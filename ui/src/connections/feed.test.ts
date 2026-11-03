import { afterEach, describe, expect, it, vi } from "vitest";
import { watchConnections } from "./feed";
import type { ConnectionSnapshot } from "./contracts";
const mock = vi.hoisted(() => ({ listen: vi.fn(), invoke: vi.fn() }));
vi.mock("../terminal/bridge", () => ({ listenTauriEvent: mock.listen, invokeTauri: mock.invoke }));
const snapshot = (generation: string, state = "online") => ({ connectionId: "80000000-0000-4000-8000-000000000001", generation, state, errorCode: null });
afterEach(() => vi.resetAllMocks());

describe("connection snapshot feed", () => {
  it("subscribes before the initial query and rejects old events and old query responses", async () => {
    let receive!: (value: unknown) => void;
    const unlisten = vi.fn(), observed: ConnectionSnapshot[] = [];
    mock.listen.mockImplementation(async (_event: string, callback: typeof receive) => { receive = callback; return unlisten; });
    mock.invoke.mockImplementationOnce(async () => { receive(snapshot("2", "connecting")); return [snapshot("1")]; }).mockResolvedValue([snapshot("1")]);
    const stop = await watchConnections((value) => observed.push(value));
    expect(mock.listen.mock.invocationCallOrder[0]).toBeLessThan(mock.invoke.mock.invocationCallOrder[0]);
    expect(observed.at(-1)).toEqual(snapshot("2", "connecting"));
    receive(snapshot("1"));
    expect(observed.at(-1)).toEqual(snapshot("2", "connecting"));
    stop();
    receive(snapshot("3", "connecting"));
    expect(observed.at(-1)).toEqual(snapshot("2", "connecting"));
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("resynchronizes a missed generation using an authoritative snapshot", async () => {
    let receive!: (value: unknown) => void;
    mock.listen.mockImplementation(async (_event: string, callback: typeof receive) => { receive = callback; return vi.fn(); });
    mock.invoke.mockResolvedValueOnce([snapshot("1")]).mockResolvedValue([snapshot("3")]);
    const observed = vi.fn();
    const stop = await watchConnections(observed);
    expect(observed).toHaveBeenLastCalledWith(snapshot("1"), "query");
    receive(snapshot("1", "reconnecting"));
    expect(observed).toHaveBeenLastCalledWith(snapshot("1", "reconnecting"), "event");
    receive(snapshot("3"));
    await Promise.resolve(); await Promise.resolve();
    expect(observed).toHaveBeenLastCalledWith(snapshot("3"), "query");
    stop();
  });

  it("unsubscribes if querying fails and ignores malformed state events", async () => {
    let receive!: (value: unknown) => void;
    const unlisten = vi.fn(), observed = vi.fn();
    mock.listen.mockImplementation(async (_event: string, callback: typeof receive) => { receive = callback; return unlisten; });
    mock.invoke.mockRejectedValue({ code: "operation_failed" });
    await expect(watchConnections(observed)).rejects.toEqual({ code: "operation_failed" });
    receive({ ...snapshot("1"), state: ["online"] });
    expect(observed).not.toHaveBeenCalled();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
