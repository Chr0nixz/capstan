import { afterEach, describe, expect, it, vi } from "vitest";
import { BenchmarkError, until, withDeadline } from "./deadline";

afterEach(() => { vi.useRealTimers(); });

describe("benchmark deadlines", () => {
  it("bounds an IPC submission that never settles without replaying it", async () => {
    vi.useFakeTimers();
    const submit = vi.fn(() => new Promise<void>(() => undefined));
    const result = withDeadline(submit, new AbortController().signal);
    const rejected = expect(result).rejects.toMatchObject({ code: "timeout" });
    await vi.advanceTimersByTimeAsync(30_000);
    await rejected;
    expect(submit).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("bounds a hung asynchronous readiness predicate", async () => {
    vi.useFakeTimers();
    const predicate = vi.fn(() => new Promise<boolean>(() => undefined));
    const result = until(predicate, new AbortController().signal, 100);
    const rejected = expect(result).rejects.toMatchObject({ code: "timeout" });
    await vi.advanceTimersByTimeAsync(100);
    await rejected;
    expect(predicate).toHaveBeenCalledTimes(1);
  });

  it("cancels while IPC is pending and stops polling after a late result", async () => {
    vi.useFakeTimers();
    const cancel = new AbortController();
    let resolve!: (value: boolean) => void;
    const predicate = vi.fn(() => new Promise<boolean>((done) => { resolve = done; }));
    const result = until(predicate, cancel.signal);
    const rejected = expect(result).rejects.toMatchObject({ code: "cancelled" });
    cancel.abort();
    await rejected;
    resolve(false);
    await vi.advanceTimersByTimeAsync(100);
    expect(predicate).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("preserves failures and releases deadline timers after success", async () => {
    vi.useFakeTimers();
    const signal = new AbortController().signal;
    const failure = new BenchmarkError("backend");
    await expect(withDeadline(() => Promise.reject(failure), signal)).rejects.toBe(failure);
    await expect(withDeadline(() => Promise.resolve(23), signal)).resolves.toBe(23);
    await until(() => true, signal);
    expect(vi.getTimerCount()).toBe(0);
  });
});
