import { createTerminalController, type ControllerCallbacks, type TerminalController, type TerminalFailure } from "../terminal/controller";
import { invokeTauri } from "../terminal/bridge";
import type { StartedTerminal, TerminalLaunch } from "../terminal/contracts";
import { createTauriTransport, type TerminalTransport } from "../terminal/transport";
import { watchConnections, type ConnectionSnapshotSource } from "./feed";
import { acceptsConnectionSnapshot, type ConnectionId, type ConnectionSnapshot, type ExactGeneration } from "./contracts";

type RecoveryCallbacks = ControllerCallbacks & {
  connection: (snapshot: ConnectionSnapshot) => void;
  suspended: () => void;
  replacing: () => void | Promise<void>;
};
type RecoveryDependencies = {
  transport: () => TerminalTransport;
  watch: typeof watchConnections;
  disconnect: (snapshot: ConnectionSnapshot) => Promise<ConnectionSnapshot>;
  reconnect: (snapshot: ConnectionSnapshot) => Promise<ConnectionSnapshot>;
};
const defaults: RecoveryDependencies = {
  transport: createTauriTransport, watch: watchConnections,
  disconnect: (snapshot) => invokeTauri("disconnect_connection", { connectionId: snapshot.connectionId, expectedGeneration: snapshot.generation }),
  reconnect: (snapshot) => invokeTauri("reconnect_connection", { connectionId: snapshot.connectionId, expectedGeneration: snapshot.generation, attemptId: crypto.randomUUID() }),
};

/** Each replacement owns new Channels, sequence numbers and input queues. */
export function createRecoveringController(launch: TerminalLaunch, callbacks: RecoveryCallbacks, dependencies: RecoveryDependencies = defaults) {
  let disposed = false, suspended = false, replacing = false, paneExited = false, version = 0;
  let started: StartedTerminal | undefined;
  let connection: ConnectionSnapshot | undefined;
  let generation: ExactGeneration | undefined;
  let unlisten: (() => void) | undefined;
  let size = { columns: 80, rows: 24 };
  let current: TerminalController;
  let replacementTask: Promise<void> | undefined;
  const acknowledgments: { controller: TerminalController; remaining: number }[] = [];
  const start = (nextLaunch: TerminalLaunch): TerminalController => {
    const identity = ++version;
    paneExited = false;
    const controller = createTerminalController(nextLaunch, {
      started: (next) => {
        if (disposed || identity !== version) return;
        started = next;
        if (next.kind === "ssh") generation = next.generation;
        suspended = false;
        callbacks.started?.(next);
      },
      output: (bytes) => {
        if (disposed || identity !== version) return;
        acknowledgments.push({ controller, remaining: bytes.byteLength });
        callbacks.output(bytes);
      },
      status: (snapshot) => {
        if (disposed || identity !== version) return;
        if (snapshot.state === "exited") paneExited = true;
        callbacks.status(snapshot);
      },
      failure: (failure) => { if (!disposed && identity === version) callbacks.failure(failure); },
    }, dependencies.transport());
    controller.resize(size.columns, size.rows);
    return controller;
  };
  const receive = (snapshot: ConnectionSnapshot, source: ConnectionSnapshotSource | "command" = "command"): void => {
    if (disposed || started?.kind !== "ssh" || snapshot.connectionId !== started.connectionId || !generation
      || BigInt(snapshot.generation) < BigInt(generation)
      || (connection !== undefined && !acceptsConnectionSnapshot(connection, snapshot)
        // Only the feed's authoritative query may fill a missed generation.
        && !(source === "query" && BigInt(snapshot.generation) > BigInt(connection.generation)))) return;
    connection = snapshot;
    callbacks.connection(snapshot);
    if (paneExited) return;
    if (snapshot.state !== "online") {
      suspended = true;
      callbacks.suspended();
      if (snapshot.state === "failed") {
        const failures: Partial<Record<NonNullable<ConnectionSnapshot["errorCode"]>, TerminalFailure>> = {
          host_key_changed: "hostKeyChanged", ssh_authentication: "authenticationFailed", vault_locked: "vaultLocked",
          ssh_private_key_invalid: "privateKeyInvalid", ssh_agent_unavailable: "agentUnavailable", ssh_agent_identity_unavailable: "agentIdentityUnavailable",
          ssh_timeout: "connectionTimedOut", canceled: "operationCanceled", invalid_input: "invalidInput", host_key_rejected: "operationCanceled",
        };
        callbacks.failure(snapshot.errorCode ? failures[snapshot.errorCode] ?? "connectionFailed" : "connectionFailed");
      }
    } else if (BigInt(snapshot.generation) > BigInt(generation) && !replacing) {
      replacing = true;
      suspended = true;
      callbacks.suspended();
      replacementTask = (async () => {
        // The supervisor has already ended every shell on the old transport.
        // Retain its pane owner until the new shell has claimed its own owner.
        const old = current;
        version += 1;
        if (disposed || launch.kind !== "ssh" || connection?.state !== "online" || connection.generation !== snapshot.generation) { await old.dispose(); return; }
        generation = snapshot.generation;
        await callbacks.replacing();
        if (disposed || connection?.state !== "online" || connection.generation !== snapshot.generation) { await old.dispose(); return; }
        current = start({ ...launch, attempt_id: crypto.randomUUID(), reconnect: {
          connectionId: snapshot.connectionId as ConnectionId, expectedGeneration: snapshot.generation,
        } });
        try { await current.ready; } finally { await old.dispose(); }
      })().catch(() => { if (!disposed) callbacks.failure("connectionFailed"); }).finally(() => {
        replacing = false;
        if (!disposed && connection && generation && connection.state === "online" && BigInt(connection.generation) > BigInt(generation)) receive(connection);
      });
    }
  };
  current = start(launch);
  const ready = current.ready.then(async () => {
    if (disposed || started?.kind !== "ssh" || !generation) return;
    const stopWatching = await dependencies.watch(receive);
    if (disposed) stopWatching(); else unlisten = stopWatching;
  }).catch(() => { if (!disposed) callbacks.failure("connectionFailed"); });
  const restartFromHost = async (): Promise<void> => {
    if (disposed || replacing || launch.kind !== "ssh") return;
    replacing = true;
    suspended = true;
    connection = undefined;
    unlisten?.(); unlisten = undefined;
    callbacks.suspended();
    const old = current;
    version += 1;
    replacementTask = (async () => {
      try {
        await callbacks.replacing();
        if (disposed) return;
        current = start({ ...launch, attempt_id: crypto.randomUUID(), reconnect: undefined });
        await current.ready;
        if (disposed) return;
        const stopWatching = await dependencies.watch(receive);
        if (disposed) stopWatching(); else unlisten = stopWatching;
      } finally { await old.dispose(); replacing = false; }
    })();
    await replacementTask;
  };
  return {
    ready,
    get ended(): boolean { return suspended || current.ended; },
    get canReconnect(): boolean { return started?.kind === "ssh" && (paneExited || !connection || connection.state === "failed" || connection.state === "disconnected"); },
    async reconnect(): Promise<void> {
      if (paneExited || !connection || connection.state === "disconnected") await restartFromHost();
      else receive(await dependencies.reconnect(connection));
    },
    async cancelRecovery(): Promise<void> { if (connection) receive(await dependencies.disconnect(connection)); },
    async sendInput(bytes: Uint8Array): Promise<void> {
      const destination = current, identity = version;
      if (disposed || suspended) throw { code: "session_closed" };
      await destination.ready;
      if (disposed || suspended || identity !== version) throw { code: "session_closed" };
      await destination.sendInput(bytes);
    },
    acknowledge(bytes: number): void {
      while (bytes > 0 && acknowledgments.length) {
        const head = acknowledgments[0], count = Math.min(bytes, head.remaining);
        head.controller.acknowledge(count);
        bytes -= count; head.remaining -= count;
        if (!head.remaining) acknowledgments.shift();
      }
    },
    resize(columns: number, rows: number): void { size = { columns, rows }; current.resize(columns, rows); },
    async dispose(): Promise<void> {
      disposed = true;
      unlisten?.();
      await current.dispose();
      await ready;
      await replacementTask;
      acknowledgments.length = 0;
    },
  };
}
