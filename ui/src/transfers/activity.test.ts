import { describe, expect, it, vi } from "vitest";
import { ACTIVE_POLL_MS, IDLE_POLL_MS, notifyTransfersChanged, onTransfersChanged, transferPollDelay } from "./activity";

describe("transfer polling cadence", () => {
  it("polls quickly only while bytes move, a conflict waits, or an action is pending", () => {
    expect(transferPollDelay([], [])).toBe(IDLE_POLL_MS);
    expect(transferPollDelay([{ state: "running" }], [])).toBe(ACTIVE_POLL_MS);
    expect(transferPollDelay([{ state: "queued" }, { state: "completed" }], [])).toBe(ACTIVE_POLL_MS);
    expect(transferPollDelay([{ state: "awaiting_conflict" }], [])).toBe(ACTIVE_POLL_MS);
    expect(transferPollDelay([{ state: "paused" }, { state: "failed" }, { state: "canceled" }], [])).toBe(IDLE_POLL_MS);
    expect(transferPollDelay([{ state: "paused" }], ["task"])).toBe(ACTIVE_POLL_MS);
  });

  it("wakes every subscriber until it unsubscribes", () => {
    const first = vi.fn(), second = vi.fn();
    const stopFirst = onTransfersChanged(first), stopSecond = onTransfersChanged(second);
    notifyTransfersChanged();
    stopFirst();
    notifyTransfersChanged();
    stopSecond();
    notifyTransfersChanged();
    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(2);
  });
});
