import { describe, expect, it, vi } from "vitest";
import { routeInput, type PaneId } from "./contracts";
import { createPaneInputs } from "./input";
import { RoutedInputQueue } from "./inputQueue";

const a = "pane-a" as PaneId, b = "pane-b" as PaneId, c = "pane-c" as PaneId;
const bytes = (text: string) => new TextEncoder().encode(text);
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((settle) => { resolve = settle; });
  return { promise, resolve };
}

describe("pane input delivery", () => {
  it("sends ordinary input only to focus even when other panes are selected", async () => {
    const inputs = createPaneInputs(), first = vi.fn(async () => undefined), second = vi.fn(async () => undefined);
    inputs.register(a, first); inputs.register(b, second);
    const confirm = vi.fn(async () => false);
    expect(await inputs.dispatch(routeInput(a, [b], false, "x\r"), bytes("x\r"), confirm)).toEqual({ deliveredPaneIds: [a], failedTargets: [] });
    expect(first).toHaveBeenCalledExactlyOnceWith(bytes("x\r"));
    expect(second).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
  });

  it("never widens an empty target set, and deduplicates explicit targets", async () => {
    const inputs = createPaneInputs(), write = vi.fn(async () => undefined), confirm = vi.fn(async () => true);
    inputs.register(a, write);
    expect(await inputs.dispatch(routeInput(a, [], true, "x\r"), bytes("x\r"), confirm)).toEqual({ deliveredPaneIds: [], failedTargets: [] });
    expect(write).not.toHaveBeenCalled();
    expect(confirm).not.toHaveBeenCalled();
    await inputs.dispatch(routeInput(b, [a, a], true, "x"), bytes("x"), confirm);
    expect(write).toHaveBeenCalledTimes(1);
  });

  it.each(["one\rtwo", "one\ntwo", "one\r\ntwo", "enter\r", "\x1b[200~one\rtwo\x1b[201~"])("gates all targets before confirmation of %j and cancels every delivery", async (text) => {
    const inputs = createPaneInputs(), first = vi.fn(async () => undefined), second = vi.fn(async () => undefined);
    inputs.register(a, first); inputs.register(b, second);
    const confirmation = deferred<boolean>();
    const task = inputs.dispatch(routeInput(c, [a, b], true, text), bytes(text), () => confirmation.promise);
    await Promise.resolve();
    expect(first).not.toHaveBeenCalled(); expect(second).not.toHaveBeenCalled();
    confirmation.resolve(false);
    expect(await task).toEqual({ deliveredPaneIds: [], failedTargets: [] });
    expect(first).not.toHaveBeenCalled(); expect(second).not.toHaveBeenCalled();
  });

  it("freezes destination IDs and endpoint owners before yielding to confirmation", async () => {
    const inputs = createPaneInputs(), first = vi.fn(async () => undefined), extra = vi.fn(async () => undefined);
    inputs.register(a, first); inputs.register(c, extra);
    const confirmation = deferred<boolean>(), route = routeInput(b, [a], true, "x\n");
    const task = inputs.dispatch(route, bytes("x\n"), () => confirmation.promise);
    route.targetPaneIds.push(c);
    confirmation.resolve(true);
    expect(await task).toEqual({ deliveredPaneIds: [a], failedTargets: [] });
    expect(first).toHaveBeenCalledTimes(1); expect(extra).not.toHaveBeenCalled();
  });

  it("reports each failed target and never replays accepted or uncertain writes", async () => {
    const inputs = createPaneInputs(), first = vi.fn(async () => undefined);
    const rejected = vi.fn(async () => { throw { code: "queue_full" }; });
    const uncertain = vi.fn(async () => { throw new Error("private terminal content"); });
    inputs.register(a, first); inputs.register(b, rejected); inputs.register(c, uncertain);
    const result = await inputs.dispatch(routeInput(a, [a, b, c], true, "x\r"), bytes("x\r"), async () => true);
    expect(result).toEqual({ deliveredPaneIds: [a], failedTargets: [{ paneId: b, errorCode: "queue_full" }, { paneId: c, errorCode: "delivery_unknown" }] });
    await Promise.resolve();
    expect([first, rejected, uncertain].map((write) => write.mock.calls.length)).toEqual([1, 1, 1]);
    expect(JSON.stringify(result)).not.toContain("private");
  });

  it("serializes concurrent direct and broadcast producers at the destination without losing bytes", async () => {
    const inputs = createPaneInputs(), gate = deferred<void>(), received: string[] = [];
    inputs.register(a, async (value) => { received.push(new TextDecoder().decode(value)); if (received.length === 1) await gate.promise; });
    const one = inputs.dispatch(routeInput(a, [], false, "中"), bytes("中"), async () => true);
    const two = inputs.dispatch(routeInput(b, [a], true, "文"), bytes("文"), async () => true);
    const three = inputs.dispatch(routeInput(a, [], false, "\r"), bytes("\r"), async () => true);
    expect(received).toEqual(["中"]);
    gate.resolve();
    await Promise.all([one, two, three]);
    expect(received).toEqual(["中", "文", "\r"]);
  });

  it("bounds queued and in-flight bytes per destination independently", async () => {
    const inputs = createPaneInputs(), gate = deferred<void>(), full = vi.fn(() => gate.promise), healthy = vi.fn(async () => undefined);
    inputs.register(a, full); inputs.register(b, healthy);
    const large = bytes("x".repeat(64 * 1024));
    const running = inputs.dispatch(routeInput(a, [], false, "x"), large, async () => true);
    expect(await inputs.dispatch(routeInput(c, [a, b], true, "y"), bytes("y"), async () => true)).toEqual({
      deliveredPaneIds: [b], failedTargets: [{ paneId: a, errorCode: "input_queue_full" }],
    });
    expect(full).toHaveBeenCalledTimes(1);
    gate.resolve(); await running;
  });

  it("drops queued successors after an uncertain write without replaying them", async () => {
    const inputs = createPaneInputs(), gate = deferred<void>();
    const write = vi.fn(async () => { await gate.promise; throw new Error("unknown"); });
    inputs.register(a, write);
    const first = inputs.dispatch(routeInput(a, [], false, "a"), bytes("a"), async () => true);
    const second = inputs.dispatch(routeInput(a, [], false, "b"), bytes("b"), async () => true);
    gate.resolve();
    expect((await first).failedTargets).toEqual([{ paneId: a, errorCode: "delivery_unknown" }]);
    expect((await second).failedTargets).toEqual([{ paneId: a, errorCode: "session_closed" }]);
    expect(write).toHaveBeenCalledTimes(1);
  });

  it("does not deliver confirmed old input into a restarted pane's new session", async () => {
    const inputs = createPaneInputs(), oldWrite = vi.fn(async () => undefined), newWrite = vi.fn(async () => undefined);
    const removeOld = inputs.register(a, oldWrite), confirmation = deferred<boolean>();
    const task = inputs.dispatch(routeInput(b, [a], true, "x\r"), bytes("x\r"), () => confirmation.promise);
    removeOld(); inputs.register(a, newWrite); removeOld();
    confirmation.resolve(true);
    expect((await task).failedTargets).toEqual([{ paneId: a, errorCode: "session_closed" }]);
    expect(oldWrite).not.toHaveBeenCalled(); expect(newWrite).not.toHaveBeenCalled();
    await inputs.dispatch(routeInput(a, [], false, "new"), bytes("new"), async () => true);
    expect(newWrite).toHaveBeenCalledExactlyOnceWith(bytes("new"));
  });

  it("cancels a confirmed route whose source has closed or become inactive", async () => {
    const inputs = createPaneInputs(), write = vi.fn(async () => undefined);
    inputs.register(a, write);
    expect(await inputs.dispatch(routeInput(b, [a], true, "x\r"), bytes("x\r"), async () => true, () => false)).toEqual({ deliveredPaneIds: [], failedTargets: [] });
    expect(write).not.toHaveBeenCalled();
  });

  it("distinguishes an in-flight close from unsent queued input and never redirects either", async () => {
    const inputs = createPaneInputs(), gate = deferred<void>(), oldWrite = vi.fn(() => gate.promise);
    const close = inputs.register(a, oldWrite);
    const inFlight = inputs.dispatch(routeInput(a, [], false, "a"), bytes("a"), async () => true);
    const queued = inputs.dispatch(routeInput(a, [], false, "b"), bytes("b"), async () => true);
    close();
    const replacement = vi.fn(async () => undefined);
    inputs.register(a, replacement);
    expect((await inFlight).failedTargets).toEqual([{ paneId: a, errorCode: "delivery_unknown" }]);
    expect((await queued).failedTargets).toEqual([{ paneId: a, errorCode: "session_closed" }]);
    gate.resolve(); await Promise.resolve();
    expect(oldWrite).toHaveBeenCalledTimes(1);
    expect(replacement).not.toHaveBeenCalled();
  });

  it("does not widen already queued ordinary input when broadcast is enabled before it drains", async () => {
    const inputs = createPaneInputs(), first = vi.fn(async (_value: Uint8Array) => undefined), second = vi.fn(async (_value: Uint8Array) => undefined);
    inputs.register(a, first); inputs.register(b, second);
    let enabled = false;
    const source = new RoutedInputQueue((value) => inputs.prepare(routeInput(a, [b], enabled, new TextDecoder().decode(value))));
    source.enqueue(bytes("one")); source.enqueue(bytes("two"));
    enabled = true;
    source.enqueue(bytes("new"));
    while (source.peek()) {
      await source.route()!(source.peek()!, async () => true);
      source.dequeue();
    }
    expect(first.mock.calls.map(([value]) => new TextDecoder().decode(value))).toEqual(["one", "two"]);
    expect(second.mock.calls.map(([value]) => new TextDecoder().decode(value))).toEqual(["new"]);
  });

  it("keeps queued broadcast destinations and their session owners through selection changes and restarts", async () => {
    const inputs = createPaneInputs(), first = vi.fn(async () => undefined), second = vi.fn(async () => undefined), replacement = vi.fn(async () => undefined);
    inputs.register(a, first); inputs.register(b, second);
    let selected = [a];
    const source = new RoutedInputQueue((value) => inputs.prepare(routeInput(c, selected, true, new TextDecoder().decode(value))));
    source.enqueue(bytes("one")); source.enqueue(bytes("two"));
    selected = [b];
    await source.route()!(source.peek()!, async () => true); source.dequeue();
    inputs.register(a, replacement);
    expect((await source.route()!(source.peek()!, async () => true)).failedTargets).toEqual([{ paneId: a, errorCode: "session_closed" }]);
    expect(first).toHaveBeenCalledExactlyOnceWith(bytes("one"));
    expect(second).not.toHaveBeenCalled(); expect(replacement).not.toHaveBeenCalled();
  });

  it("does not retain rejected or cleared source routes when accepting fresh input", async () => {
    const inputs = createPaneInputs(), first = vi.fn(async () => undefined), second = vi.fn(async () => undefined);
    inputs.register(a, first); inputs.register(b, second);
    let selected = [a];
    const source = new RoutedInputQueue((value) => inputs.prepare(routeInput(c, selected, true, new TextDecoder().decode(value))));
    expect(source.enqueue(bytes("x".repeat(64 * 1024))).accepted).toBe(true);
    expect(source.enqueue(bytes("rejected")).accepted).toBe(false);
    source.clear(); selected = [b];
    source.enqueue(bytes("fresh"));
    await source.route()!(source.peek()!, async () => true); source.dequeue();
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledExactlyOnceWith(bytes("fresh"));
    expect(source.route()).toBeUndefined();
    source.close();
    expect(source.enqueue(bytes("closed")).accepted).toBe(false);
  });
});
