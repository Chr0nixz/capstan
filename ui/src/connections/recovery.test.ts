import { afterEach, describe, expect, it, vi } from "vitest";
import { createRecoveringController } from "./controller";
import { watchConnections } from "./feed";
import type { ConnectionId, ConnectionSnapshot, ExactGeneration } from "./contracts";
import type { HostId, StartedTerminal, TerminalLaunch, TerminalSessionId } from "../terminal/contracts";
import type { TerminalTransport } from "../terminal/transport";

const bridge = vi.hoisted(() => ({ listen: vi.fn(), invoke: vi.fn() }));
vi.mock("../terminal/bridge", () => ({ listenTauriEvent: bridge.listen, invokeTauri: bridge.invoke }));

const connectionId = "80000000-0000-4000-8000-000000000001" as ConnectionId;
const hostId = "11111111-1111-4111-8111-111111111111" as HostId;
const snapshot = (generation: string, state: ConnectionSnapshot["state"]): ConnectionSnapshot => ({
  connectionId, generation: generation as ExactGeneration, state,
  errorCode: state === "reconnecting" ? "connection_lost" : null,
});
const controllers: ReturnType<typeof createRecoveringController>[] = [];
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function setup() {
  let emit!: (value: unknown) => void;
  const unlisten = vi.fn();
  bridge.listen.mockImplementation(async (_event: string, callback: typeof emit) => { emit = callback; return unlisten; });
  bridge.invoke.mockResolvedValue([snapshot("1", "online")]);
  const transports: TerminalTransport[] = [], starts: TerminalLaunch[] = [];
  const callbacks = { started: vi.fn(), output: vi.fn(), status: vi.fn(), failure: vi.fn(), suspended: vi.fn(), replacing: vi.fn(), connection: vi.fn() };
  const reconnect = vi.fn(async (value: ConnectionSnapshot) => value);
  const controller = createRecoveringController({ kind: "ssh", host_id: hostId, attempt_id: "initial", theme: "graphite" }, callbacks, {
    watch: watchConnections, reconnect, disconnect: async (value) => ({ ...value, state: "disconnected" }),
    transport: () => {
      const index = transports.length;
      const transport: TerminalTransport = {
        start: vi.fn(async (launch: TerminalLaunch): Promise<StartedTerminal> => {
          starts.push(launch);
          return {
            kind: "ssh", id: `session-${index}` as TerminalSessionId, hostId, connectionId,
            generation: (launch.kind === "ssh" ? launch.reconnect?.expectedGeneration ?? "1" : "1") as ExactGeneration,
          };
        }),
        input: vi.fn(async () => undefined), ack: vi.fn(async () => undefined), resize: vi.fn(async () => undefined),
        stop: vi.fn(async () => undefined), cancelAttempt: vi.fn(async () => undefined), detach: vi.fn(),
      };
      transports.push(transport);
      return transport;
    },
  });
  controllers.push(controller);
  return { controller, callbacks, transports, starts, reconnect, unlisten, emit: (value: unknown) => emit(value) };
}

afterEach(async () => {
  await Promise.all(controllers.splice(0).map((controller) => controller.dispose()));
  vi.resetAllMocks();
});

describe("connection feed and recovery controller", () => {
  it.each(["2", "3", "9007199254740993"])("recovers generation %s after missing connecting events without replaying input", async (generation) => {
    const test = setup(); await test.controller.ready;
    await test.controller.sendInput(new Uint8Array([65]));
    test.emit(snapshot("1", "reconnecting"));
    const query = deferred<ConnectionSnapshot[]>();
    bridge.invoke.mockReturnValueOnce(query.promise);
    test.emit(snapshot(generation, "online"));
    expect(bridge.invoke).toHaveBeenCalledTimes(2);
    expect(test.starts).toHaveLength(1);
    await expect(test.controller.sendInput(new Uint8Array([66]))).rejects.toEqual({ code: "session_closed" });

    query.resolve([snapshot(generation, "online")]);
    await tick();
    expect(test.starts).toHaveLength(2);
    expect(test.starts[1]).toMatchObject({ reconnect: { connectionId, expectedGeneration: generation } });
    expect(test.controller.ended).toBe(false);
    expect(test.callbacks.connection).toHaveBeenLastCalledWith(snapshot(generation, "online"));
    test.emit(snapshot(generation, "online"));
    await tick();
    expect(test.starts).toHaveLength(2);
    await test.controller.sendInput(new Uint8Array([67]));
    expect(test.transports[0].input).toHaveBeenCalledExactlyOnceWith("session-0", new Uint8Array([65]));
    expect(test.transports[1].input).toHaveBeenCalledExactlyOnceWith("session-1", new Uint8Array([67]));
    expect(test.callbacks.replacing).toHaveBeenCalledTimes(1);
  });

  it.each(["2", "3"])("rejects a late generation %s command reply after an authoritative resync", async (generation) => {
    const test = setup(); await test.controller.ready;
    test.emit(snapshot("1", "reconnecting"));
    test.emit(snapshot("1", "failed"));
    const reply = deferred<ConnectionSnapshot>();
    test.reconnect.mockReturnValueOnce(reply.promise);
    const reconnecting = test.controller.reconnect();
    bridge.invoke.mockResolvedValueOnce([snapshot("3", "online")]);
    test.emit(snapshot("3", "online"));
    await tick();
    expect(test.starts).toHaveLength(2);

    reply.resolve(snapshot(generation, "connecting"));
    await reconnecting;
    expect(test.callbacks.connection).toHaveBeenLastCalledWith(snapshot("3", "online"));
    expect(test.controller.ended).toBe(false);
    await test.controller.sendInput(new Uint8Array([67]));
    expect(test.transports[1].input).toHaveBeenCalledExactlyOnceWith("session-1", new Uint8Array([67]));
  });

  it("ignores a stale authoritative query after a newer generation has recovered", async () => {
    const test = setup(); await test.controller.ready;
    test.emit(snapshot("1", "reconnecting"));
    const oldQuery = deferred<ConnectionSnapshot[]>();
    bridge.invoke.mockReturnValueOnce(oldQuery.promise).mockResolvedValueOnce([snapshot("4", "online")]);
    test.emit(snapshot("3", "online"));
    test.emit(snapshot("4", "online"));
    await tick();
    oldQuery.resolve([snapshot("3", "online")]);
    await tick();
    expect(test.starts).toHaveLength(2);
    expect(test.starts[1]).toMatchObject({ reconnect: { expectedGeneration: "4" } });
    expect(test.callbacks.connection).toHaveBeenLastCalledWith(snapshot("4", "online"));
    expect(test.controller.ended).toBe(false);
  });

  it("requires an authoritative query before accepting a command reply that skips connecting", async () => {
    const test = setup(); await test.controller.ready;
    test.emit(snapshot("1", "reconnecting"));
    test.emit(snapshot("1", "failed"));
    test.reconnect.mockResolvedValueOnce(snapshot("3", "online"));
    await test.controller.reconnect();
    expect(test.starts).toHaveLength(1);
    expect(test.controller.ended).toBe(true);
    expect(test.callbacks.connection).toHaveBeenLastCalledWith(snapshot("1", "failed"));

    bridge.invoke.mockResolvedValueOnce([snapshot("3", "online")]);
    test.emit(snapshot("3", "online"));
    await tick();
    expect(test.starts).toHaveLength(2);
    expect(test.controller.ended).toBe(false);
  });

  it("uses the latest authoritative generation when another recovery overtakes the parser boundary", async () => {
    const test = setup(); await test.controller.ready;
    test.emit(snapshot("1", "reconnecting"));
    const boundary = deferred<void>();
    test.callbacks.replacing.mockReturnValueOnce(boundary.promise);
    bridge.invoke.mockResolvedValueOnce([snapshot("3", "online")]).mockResolvedValueOnce([snapshot("4", "online")]);
    test.emit(snapshot("3", "online"));
    await tick();
    expect(test.starts).toHaveLength(1);
    test.emit(snapshot("4", "online"));
    await tick();
    boundary.resolve();
    await tick();
    expect(test.starts).toHaveLength(2);
    expect(test.starts[1]).toMatchObject({ reconnect: { expectedGeneration: "4" } });
    expect(test.controller.ended).toBe(false);
    expect(test.transports[0].stop).toHaveBeenCalledTimes(1);
  });

  it("does not start a shell for a query completing after disposal", async () => {
    const test = setup(); await test.controller.ready;
    test.emit(snapshot("1", "reconnecting"));
    const query = deferred<ConnectionSnapshot[]>();
    bridge.invoke.mockReturnValueOnce(query.promise);
    test.emit(snapshot("3", "online"));
    await test.controller.dispose();
    query.resolve([snapshot("3", "online")]);
    await tick();
    expect(test.starts).toHaveLength(1);
    expect(test.unlisten).toHaveBeenCalledTimes(1);
  });
});
