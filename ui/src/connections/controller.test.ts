import { describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/terminal-contract.json";
import { createRecoveringController } from "./controller";
import type { watchConnections } from "./feed";
import type { ConnectionId, ConnectionSnapshot, ExactGeneration } from "./contracts";
import type { HostId, StartedTerminal, TerminalLaunch, TerminalSessionId, TerminalSnapshot } from "../terminal/contracts";
import type { TerminalTransport } from "../terminal/transport";

const connectionId = "80000000-0000-4000-8000-000000000001" as ConnectionId;
const generation = (value: number) => String(value) as ExactGeneration;
const launch: Extract<TerminalLaunch, {kind: "ssh"}> = { kind: "ssh", host_id: "host" as HostId, attempt_id: "first", theme: "graphite" };
const snapshot = (value: number, state: ConnectionSnapshot["state"]): ConnectionSnapshot => ({
  connectionId, generation: generation(value), state, errorCode: state === "reconnecting" ? "connection_lost" : null,
});
function setup() {
  const transports: (TerminalTransport & {
    output: (frame: ArrayBuffer) => void;
    status: (value: TerminalSnapshot) => void;
  })[] = [];
  const starts: TerminalLaunch[] = [];
  let receive!: Parameters<typeof watchConnections>[0];
  const watch = vi.fn(async (callback: typeof receive) => { receive = callback; return vi.fn(); });
  const callbacks = { started: vi.fn(), output: vi.fn(), status: vi.fn(), failure: vi.fn(), suspended: vi.fn(), replacing: vi.fn(), connection: vi.fn() };
  const disconnect = vi.fn(async (value: ConnectionSnapshot) => ({ ...value, state: "disconnected" as const }));
  const reconnect = vi.fn(async (value: ConnectionSnapshot) => snapshot(Number(value.generation) + 1, "connecting"));
  const controller = createRecoveringController(launch, callbacks, {
    watch, disconnect, reconnect,
    transport: () => {
      const index = transports.length;
      const transport = {
        output: (_frame: ArrayBuffer): void => undefined, status: (_value: TerminalSnapshot): void => undefined,
        start: vi.fn(async (next: TerminalLaunch, output: typeof transport.output, status: typeof transport.status): Promise<StartedTerminal> => {
          starts.push(next); transport.output = output; transport.status = status;
          return { kind: "ssh", id: `session-${index}` as TerminalSessionId, hostId: launch.host_id, connectionId, generation: generation(index + 1) };
        }),
        input: vi.fn(async () => undefined), ack: vi.fn(async () => undefined), resize: vi.fn(async () => undefined),
        stop: vi.fn(async () => undefined), cancelAttempt: vi.fn(async () => undefined), detach: vi.fn(),
      };
      transports.push(transport);
      return transport;
    },
  });
  return { controller, transports, starts, callbacks, watch, disconnect, reconnect, receive: (value: ConnectionSnapshot) => receive(value, "event") };
}
const frame = () => { const data = new Uint8Array(fixture.encoded); new DataView(data.buffer).setBigUint64(4, 0n, true); return data.buffer; };
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("connection recovery controller", () => {
  it("waits for the old parser boundary and honors disposal before a replacement starts", async () => {
    const test = setup(); await test.controller.ready;
    let release!: () => void;
    test.callbacks.replacing.mockImplementation(() => new Promise<void>((resolve) => { release = resolve; }));
    test.receive(snapshot(2, "online"));
    await expect(test.controller.sendInput(new Uint8Array([65]))).rejects.toEqual({ code: "session_closed" });
    expect(test.transports).toHaveLength(1);
    const disposed = test.controller.dispose();
    release();
    await disposed;
    expect(test.transports).toHaveLength(1);
    expect(test.transports[0].stop).toHaveBeenCalledTimes(1);
  });
  it("replaces the session with fresh Channels, drops disconnected input and ignores old events", async () => {
    const test = setup();
    await test.controller.ready;
    await test.controller.sendInput(new Uint8Array([65]));
    test.receive(snapshot(1, "reconnecting"));
    await expect(test.controller.sendInput(new Uint8Array([66]))).rejects.toEqual({ code: "session_closed" });
    test.receive(snapshot(2, "connecting"));
    test.receive(snapshot(2, "online"));
    await tick();
    expect(test.transports).toHaveLength(2);
    expect(test.starts[1]).toMatchObject({ reconnect: { connectionId, expectedGeneration: "2" } });
    expect(test.starts[1].kind === "ssh" && test.starts[1].attempt_id).not.toBe("first");
    expect(test.transports[0].stop).toHaveBeenCalledExactlyOnceWith("session-0");
    test.transports[0].output(frame());
    test.transports[0].status({ ...fixture.snapshot, id: "session-0" as TerminalSessionId } as TerminalSnapshot);
    expect(test.callbacks.output).not.toHaveBeenCalled();
    expect(test.callbacks.status).not.toHaveBeenCalled();
    test.transports[1].output(frame());
    expect(test.callbacks.output).toHaveBeenCalledTimes(1);
    await test.controller.sendInput(new Uint8Array([67]));
    expect(test.transports[0].input).toHaveBeenCalledExactlyOnceWith("session-0", new Uint8Array([65]));
    expect(test.transports[1].input).toHaveBeenCalledExactlyOnceWith("session-1", new Uint8Array([67]));
    expect(test.callbacks.replacing).toHaveBeenCalledTimes(1);
    expect(test.callbacks.failure).not.toHaveBeenCalled();
    await test.controller.dispose();
  });

  it("keeps a naturally exited pane closed across shared reconnects until explicitly restarted", async () => {
    const test = setup(); await test.controller.ready;
    test.transports[0].status({
      ...fixture.snapshot,
      id: "session-0" as TerminalSessionId,
      state: "exited",
      exit_code: 0,
      metrics: { ...fixture.snapshot.metrics, output_bytes: 0 },
    } as TerminalSnapshot);
    test.receive(snapshot(2, "connecting"));
    test.receive(snapshot(2, "online"));
    await tick();
    expect(test.transports).toHaveLength(1);
    expect(test.controller.ended).toBe(true);
    expect(test.controller.canReconnect).toBe(true);

    await test.controller.reconnect();
    expect(test.reconnect).not.toHaveBeenCalled();
    expect(test.transports).toHaveLength(2);
    expect(test.starts[1]).toMatchObject({ kind: "ssh", host_id: launch.host_id, reconnect: undefined });
    expect(test.controller.ended).toBe(false);
    await test.controller.dispose();
  });

  it("ignores a late reconnect command reply that regresses the accepted generation state", async () => {
    const test = setup(); await test.controller.ready;
    test.receive(snapshot(1, "failed"));
    const reply = deferred<ConnectionSnapshot>();
    test.reconnect.mockReturnValueOnce(reply.promise);
    const reconnecting = test.controller.reconnect();

    test.receive(snapshot(2, "connecting"));
    test.receive(snapshot(2, "online"));
    await tick();
    expect(test.transports).toHaveLength(2);
    reply.resolve(snapshot(2, "connecting"));
    await reconnecting;
    await tick();

    expect(test.callbacks.connection).toHaveBeenLastCalledWith(snapshot(2, "online"));
    expect(test.controller.ended).toBe(false);
    expect(test.controller.canReconnect).toBe(false);
    await test.controller.dispose();
  });

  it("keeps late parser ACKs associated with their original session", async () => {
    const test = setup(); await test.controller.ready;
    test.transports[0].output(frame());
    test.receive(snapshot(1, "reconnecting"));
    test.receive(snapshot(2, "connecting"));
    test.receive(snapshot(2, "online"));
    await tick();
    test.transports[1].output(frame());
    const payloadLength = fixture.payload.length;
    test.controller.acknowledge(payloadLength);
    await tick();
    expect(test.transports[1].ack).not.toHaveBeenCalled();
    test.controller.acknowledge(payloadLength);
    await tick();
    expect(test.transports[1].ack).toHaveBeenCalledExactlyOnceWith("session-1", payloadLength);
    await test.controller.dispose();
  });

  it("cancels recovery at the current generation and never opens a shell for late online events", async () => {
    const test = setup(); await test.controller.ready;
    test.receive(snapshot(2, "connecting"));
    await test.controller.cancelRecovery();
    expect(test.disconnect).toHaveBeenCalledExactlyOnceWith(snapshot(2, "connecting"));
    await test.controller.dispose();
    test.receive(snapshot(3, "online"));
    await tick();
    expect(test.transports).toHaveLength(1);
  });

  it("starts a user-requested new host session after cancellation without replacing the screen owner", async () => {
    const test = setup(); await test.controller.ready;
    test.receive(snapshot(1, "reconnecting"));
    await test.controller.cancelRecovery();
    expect(test.controller.canReconnect).toBe(true);
    await test.controller.reconnect();
    expect(test.transports).toHaveLength(2);
    expect(test.starts[1]).toMatchObject({ kind: "ssh", host_id: launch.host_id, reconnect: undefined });
    expect(test.watch).toHaveBeenCalledTimes(2);
    expect(test.callbacks.replacing).toHaveBeenCalledTimes(1);
    expect(test.callbacks.started).toHaveBeenCalledTimes(2);
    expect(test.transports[1].input).not.toHaveBeenCalled();
    await test.controller.dispose();
  });

  it("does not reuse a stale generation or a different logical connection", async () => {
    const test = setup(); await test.controller.ready;
    test.receive({ ...snapshot(3, "online"), connectionId: "90000000-0000-4000-8000-000000000001" as ConnectionId });
    test.receive(snapshot(1, "online"));
    await tick();
    expect(test.transports).toHaveLength(1);
    test.receive(snapshot(1, "reconnecting"));
    test.receive(snapshot(1, "failed"));
    expect(test.controller.canReconnect).toBe(true);
    await test.controller.reconnect();
    expect(test.reconnect).toHaveBeenCalledExactlyOnceWith(snapshot(1, "failed"));
    expect(test.controller.ended).toBe(true);
    await test.controller.dispose();
  });
});
