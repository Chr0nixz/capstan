import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TerminalSessionId } from "../terminal/contracts";
import { boundedCwdHint, createCwdHintHub } from "./cwdHints";

const terminal = "terminal-1" as TerminalSessionId;
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("bounded terminal directory hints", () => {
  it("coalesces a flood to one latest value every 250 ms without awaiting consumers", async () => {
    const hub = createCwdHintHub(), publisher = hub.open(terminal), listener = vi.fn();
    const unsubscribe = hub.subscribe(terminal, listener);
    listener.mockClear();
    for (let i = 0; i < 10000; i++) publisher.publish(`file://host/${i}`);
    expect(listener).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(250);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(listener).toHaveBeenLastCalledWith({ sequence: "10000", uri: "file://host/9999" }, null);
    publisher.publish("file://host/next");
    await vi.advanceTimersByTimeAsync(249);
    expect(listener).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(listener).toHaveBeenCalledTimes(2);
    unsubscribe(); publisher.close();
  });

  it("bounds UTF-8 bytes, rejects lossy surrogates, and releases a closed source's timer", async () => {
    expect(boundedCwdHint("中".repeat(2731))).toBe(false);
    expect(boundedCwdHint("\ud800")).toBe(false);
    expect(boundedCwdHint("x".repeat(8192))).toBe(true);
    const hub = createCwdHintHub(), publisher = hub.open(terminal), listener = vi.fn();
    const unsubscribe = hub.subscribe(terminal, listener);
    publisher.publish("file://host/late"); publisher.close();
    listener.mockClear();
    publisher.publish("file://host/after-close");
    await vi.advanceTimersByTimeAsync(1000);
    expect(listener).not.toHaveBeenCalled();
    unsubscribe();
  });

  it("isolates terminal IDs and refuses a second owner of the same source", async () => {
    const hub = createCwdHintHub(), first = hub.open(terminal), duplicate = hub.open(terminal);
    const other = hub.open("terminal-2" as TerminalSessionId), listener = vi.fn();
    const unsubscribe = hub.subscribe(terminal, listener);
    listener.mockClear(); duplicate.publish("file://other/wrong"); other.publish("file://other/hidden");
    first.publish("file://host/correct");
    await vi.advanceTimersByTimeAsync(250);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(listener.mock.calls[0][0].uri).toBe("file://host/correct");
    duplicate.close(); first.close(); other.close(); unsubscribe();
  });
});
