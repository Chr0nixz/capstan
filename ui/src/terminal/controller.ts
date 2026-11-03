import { decodeTerminalFrame } from "./frame";
import type { StartedTerminal, TerminalLaunch, TerminalSessionId, TerminalSnapshot } from "./contracts";
import { createTauriTransport, type TerminalTransport } from "./transport";

export type TerminalFailure =
  | "startFailed" | "outputInvalid" | "operationFailed" | "hostKeyChanged"
  | "authenticationFailed" | "connectionFailed" | "connectionTimedOut" | "shellRejected"
  | "operationCanceled" | "privateKeyInvalid" | "credentialFileInvalid"
  | "agentUnavailable" | "agentIdentityUnavailable" | "vaultLocked" | "invalidInput" | "workingDirectoryInvalid";
export type TerminalController = ReturnType<typeof createTerminalController>;
export type ControllerCallbacks = {
  started?: (started: StartedTerminal) => void;
  output: (bytes: Uint8Array) => void;
  status: (snapshot: TerminalSnapshot) => void;
  failure: (failure: TerminalFailure) => void;
};

/** Owns one backend session independently of React labels and layout. */
export function createTerminalController(
  launch: TerminalLaunch, callbacks: ControllerCallbacks, transport: TerminalTransport = createTauriTransport(),
) {
  let id: TerminalSessionId | undefined;
  let disposed = false;
  let failed = false;
  let ended = false;
  let sequence = 0n;
  let buffered: Uint8Array[] = [];
  let bufferedBytes = 0;
  let pendingAck = 0;
  let acking = false;
  let resizing = false;
  let pendingSize: { columns: number; rows: number } | undefined;
  let stopTask: Promise<void> | undefined;
  let initialStatus: TerminalSnapshot | undefined;
  let receivedBytes = 0;
  let exitStatus: TerminalSnapshot | undefined;
  const stop = (): Promise<void> => {
    if (!id) return Promise.resolve();
    stopTask ??= transport.stop(id);
    return stopTask;
  };
  const fail = (failure: TerminalFailure): void => {
    if (failed || disposed) return;
    failed = true;
    buffered = [];
    bufferedBytes = 0;
    callbacks.failure(failure);
    void stop().catch(() => undefined);
  };
  const flushResize = async (): Promise<void> => {
    if (resizing || !id || ended || disposed || failed) return;
    resizing = true;
    try {
      while (pendingSize && !disposed && !failed && !ended) {
        const size = pendingSize;
        pendingSize = undefined;
        await transport.resize(id, size.columns, size.rows);
      }
    } catch {
      if (!ended) fail("operationFailed");
    } finally { resizing = false; }
  };
  const flushAck = async (): Promise<void> => {
    if (acking || !id || disposed || failed) return;
    acking = true;
    try {
      while (pendingAck > 0 && !disposed && !failed) {
        const bytes = pendingAck;
        pendingAck = 0;
        await transport.ack(id, bytes);
      }
    } catch { fail("operationFailed"); }
    finally { acking = false; }
  };
  // Raw output frames reach the webview through an extra async fetch, while
  // small JSON status messages are delivered inline, so the final status can
  // overtake the tail of the output. A clean exit is reported only after every
  // byte it accounts for has been delivered. Error states may never deliver
  // their pending output and are reported immediately.
  const reportStatus = (snapshot: TerminalSnapshot): void => {
    if (snapshot.state !== "exited") { callbacks.status(snapshot); return; }
    exitStatus = snapshot;
    releaseExitStatus();
  };
  const releaseExitStatus = (): void => {
    if (!exitStatus || receivedBytes < exitStatus.metrics.output_bytes) return;
    const snapshot = exitStatus;
    exitStatus = undefined;
    callbacks.status(snapshot);
  };
  const ready = transport.start(launch, (message) => {
    if (disposed || failed) return;
    try {
      const frame = decodeTerminalFrame(message);
      if (frame.sequence !== sequence) throw new Error("sequence_mismatch");
      sequence += 1n;
      receivedBytes += frame.payload.byteLength;
      if (id) callbacks.output(frame.payload);
      else {
        bufferedBytes += frame.payload.byteLength;
        if (bufferedBytes > 1024 * 1024 + 64 * 1024) throw new Error("prestart_overflow");
        buffered.push(frame.payload);
      }
    } catch { fail("outputInvalid"); return; }
    if (id) releaseExitStatus();
  }, (snapshot) => {
    if (disposed || failed) return;
    if (id && id !== snapshot.id) { fail("outputInvalid"); return; }
    ended = snapshot.state !== "running";
    if (id) reportStatus(snapshot);
    else initialStatus = snapshot;
  }).then(async (started) => {
    id = started.id;
    if (disposed || failed) { await stop(); return; }
    callbacks.started?.(started);
    for (const bytes of buffered) callbacks.output(bytes);
    buffered = [];
    bufferedBytes = 0;
    if (initialStatus) {
      if (initialStatus.id !== id) { fail("outputInvalid"); return; }
      reportStatus(initialStatus);
    }
    void flushResize();
    void flushAck();
  }).catch((cause: unknown) => {
    const code = typeof cause === "object" && cause !== null && "code" in cause && typeof cause.code === "string" ? cause.code : undefined;
    if (code === "host_key_changed") fail("hostKeyChanged");
    else if (code === "ssh_authentication") fail("authenticationFailed");
    else if (code === "canceled" || code === "cancelled" || code === "host_key_rejected") fail("operationCanceled");
    else if (code === "ssh_timeout") fail("connectionTimedOut");
    else if (code === "ssh_channel_rejected") fail("shellRejected");
    else if (code === "ssh_private_key_invalid") fail("privateKeyInvalid");
    else if (code === "credential_file") fail("credentialFileInvalid");
    else if (code === "ssh_agent_unavailable") fail("agentUnavailable");
    else if (code === "ssh_agent_identity_unavailable") fail("agentIdentityUnavailable");
    else if (code === "vault_locked") fail("vaultLocked");
    else if (code === "invalid_input") fail("invalidInput");
    else if (code === "working_directory_invalid") fail("workingDirectoryInvalid");
    else if (code?.startsWith("ssh_")) fail("connectionFailed");
    else fail(launch.kind === "ssh" ? "connectionFailed" : "startFailed");
  });

  return {
    ready,
    /** True once the backend reports a final state, possibly before its status callback. */
    get ended(): boolean { return ended; },
    async sendInput(bytes: Uint8Array): Promise<void> {
      await ready;
      if (!id || disposed || failed || ended) throw new Error("session_closed");
      // A rejected write may have reached the backend. Never replay it.
      await transport.input(id, bytes);
    },
    acknowledge(bytes: number): void { if (!disposed && !failed) { pendingAck += bytes; void flushAck(); } },
    resize(columns: number, rows: number): void { pendingSize = { columns, rows }; void flushResize(); },
    async dispose(): Promise<void> {
      disposed = true;
      if (!id && launch.kind === "ssh") void transport.cancelAttempt(launch.attempt_id).catch(() => undefined);
      buffered = [];
      bufferedBytes = 0;
      pendingAck = 0;
      transport.detach();
      await ready;
      await stop();
    },
  };
}
