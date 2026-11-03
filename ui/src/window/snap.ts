import { invokeTauri, listenSnapEvent } from "../terminal/bridge";
import type { SnapBounds } from "../terminal/contracts";

export const MAXIMIZE_BUTTON_ID = "maximize-window";
export type SnapTransport = {
  update: (bounds: SnapBounds) => Promise<void>;
  detach: () => Promise<void>;
  listen: typeof listenSnapEvent;
};
const nativeTransport: SnapTransport = {
  update: (bounds) => invokeTauri("plugin:snap-layout|update_snap_bounds", { bounds }),
  detach: () => invokeTauri("plugin:snap-layout|detach_snap_bounds"),
  listen: listenSnapEvent,
};
type Ownership = { owner?: symbol; task: Promise<void> };
const ownership = new WeakMap<SnapTransport, Ownership>();

/** Owns one native hitbox; geometry never depends on host names or tab state. */
export function createSnapController(button: HTMLElement, onFailure: () => void, transport = nativeTransport) {
  const lease = Symbol("snap-owner");
  const shared = ownership.get(transport) ?? { task: Promise.resolve() };
  ownership.set(transport, shared);
  shared.owner = lease;
  const nativeOperation = (operation: () => Promise<void>): Promise<void> => {
    const result = shared.task.then(async () => { if (shared.owner === lease) await operation(); });
    shared.task = result.catch(() => undefined);
    return result;
  };
  let disposed = false;
  let failed = false;
  let pending: SnapBounds | undefined;
  let task: Promise<void> | undefined;
  let disposeTask: Promise<void> | undefined;
  let failureTask: Promise<void> | undefined;
  let last = "";
  let resolution: MediaQueryList | undefined;
  const unlisteners: Array<() => void> = [];
  const failure = (): Promise<void> => {
    if (disposed) return Promise.resolve();
    if (!failed) { failed = true; onFailure(); }
    button.classList.remove("is-hovered");
    // Failed subscriptions must also release the overlay for DOM interaction.
    failureTask ??= nativeOperation(() => transport.detach()).catch(() => undefined);
    return failureTask;
  };
  const flush = (): void => {
    if (task || disposed || failed) return;
    task = (async () => {
      try {
        while (pending && !disposed && !failed) {
          const bounds = pending;
          pending = undefined;
          await nativeOperation(() => transport.update(bounds));
        }
      } catch {
        await failure();
      }
    })().finally(() => { task = undefined; if (pending && !disposed && !failed) flush(); });
  };
  const update = (): void => {
    if (disposed || failed) return;
    const rect = button.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    const bounds = { x: rect.left, y: rect.top, width: rect.width, height: rect.height, scale: window.devicePixelRatio || 1 };
    const signature = JSON.stringify(bounds);
    if (signature === last) return;
    last = signature;
    pending = bounds;
    flush();
  };
  const onResolutionChange = (): void => {
    resolution?.removeEventListener("change", onResolutionChange);
    resolution = window.matchMedia?.(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    resolution?.addEventListener("change", onResolutionChange);
    update();
  };
  const listening = Promise.all([
    transport.listen("tauri-snap://snap/mouseenter", () => { if (!disposed) button.classList.add("is-hovered"); }),
    transport.listen("tauri-snap://snap/mouseleave", () => button.classList.remove("is-hovered")),
  ].map(async (subscription) => {
    try {
      const unlisten = await subscription;
      if (disposed) unlisten(); else unlisteners.push(unlisten);
    } catch { await failure(); }
  }));
  const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(update);
  observer?.observe(button);
  observer?.observe(document.documentElement);
  window.addEventListener("resize", update);
  onResolutionChange();
  return {
    update,
    dispose(): Promise<void> {
      disposeTask ??= (async () => {
        disposed = true;
        pending = undefined;
        observer?.disconnect();
        window.removeEventListener("resize", update);
        resolution?.removeEventListener("change", onResolutionChange);
        await listening;
        unlisteners.forEach((unlisten) => unlisten());
        unlisteners.length = 0;
        // Serialize detach after the last admitted update, including late replies.
        await task;
        button.classList.remove("is-hovered");
        await nativeOperation(() => transport.detach());
        if (shared.owner === lease) shared.owner = undefined;
      })();
      return disposeTask;
    },
  };
}
