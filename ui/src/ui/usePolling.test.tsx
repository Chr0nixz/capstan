import { act, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { usePolling } from "./usePolling";

Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });

let container: HTMLDivElement;
let root: Root;
let visibility: DocumentVisibilityState = "visible";
beforeEach(() => {
  vi.useFakeTimers();
  visibility = "visible";
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

function Poller({ refresh, delay, subscribe }: { refresh: () => unknown; delay: () => number; subscribe?: (wake: () => void) => () => void }): ReactElement {
  usePolling(refresh, delay, { subscribe });
  return <></>;
}

describe("usePolling", () => {
  it("refreshes on mount and then at the delay the owner asks for", async () => {
    const refresh = vi.fn();
    let delay = 250;
    await act(async () => root.render(<Poller refresh={refresh} delay={() => delay} />));
    expect(refresh).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(250); });
    expect(refresh).toHaveBeenCalledTimes(2);
    delay = 2_000;
    await act(async () => { await vi.advanceTimersByTimeAsync(250); });
    expect(refresh).toHaveBeenCalledTimes(3);
    await act(async () => { await vi.advanceTimersByTimeAsync(1_999); });
    expect(refresh).toHaveBeenCalledTimes(3);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(refresh).toHaveBeenCalledTimes(4);
  });

  it("pauses while the page is hidden and refreshes as soon as it is shown", async () => {
    const refresh = vi.fn();
    await act(async () => root.render(<Poller refresh={refresh} delay={() => 250} />));
    visibility = "hidden";
    await act(async () => { await vi.advanceTimersByTimeAsync(5_000); });
    expect(refresh).toHaveBeenCalledTimes(1);
    visibility = "visible";
    await act(async () => { document.dispatchEvent(new Event("visibilitychange")); await vi.advanceTimersByTimeAsync(0); });
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("refreshes immediately when woken and never overlaps calls", async () => {
    let wake: () => void = () => undefined;
    let finish: () => void = () => undefined;
    const refresh = vi.fn(() => new Promise<void>((resolve) => { finish = resolve; }));
    const subscribe = (listener: () => void): (() => void) => { wake = listener; return () => { wake = () => undefined; }; };
    await act(async () => root.render(<Poller refresh={refresh} delay={() => 10_000} subscribe={subscribe} />));
    expect(refresh).toHaveBeenCalledTimes(1);
    await act(async () => { wake(); wake(); await vi.advanceTimersByTimeAsync(0); });
    expect(refresh).toHaveBeenCalledTimes(1);
    await act(async () => { finish(); await vi.advanceTimersByTimeAsync(0); });
    expect(refresh).toHaveBeenCalledTimes(2);
    await act(async () => { finish(); await vi.advanceTimersByTimeAsync(9_999); });
    expect(refresh).toHaveBeenCalledTimes(2);
  });
});
