import { useEffect, useRef } from "react";

type Wake = (wake: () => void) => () => void;

/**
 * Runs `refresh` now and again after `nextDelay()` milliseconds, one call at a
 * time. Polling stops while the page is hidden and resumes with an immediate
 * refresh when it is shown. `subscribe` lets other code ask for an immediate
 * refresh, such as right after a transfer is queued.
 */
export function usePolling(refresh: () => unknown, nextDelay: () => number, options: { enabled?: boolean; subscribe?: Wake } = {}): void {
  const { enabled = true, subscribe } = options;
  const refreshRef = useRef(refresh);
  const delayRef = useRef(nextDelay);
  useEffect(() => { refreshRef.current = refresh; delayRef.current = nextDelay; });
  useEffect(() => {
    if (!enabled) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let running = false, again = false, disposed = false;
    const schedule = (delay: number): void => {
      if (disposed) return;
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(() => void run(), delay);
    };
    const run = async (): Promise<void> => {
      timer = undefined;
      if (disposed) return;
      if (running) { again = true; return; }
      if (document.visibilityState === "hidden") return;
      running = true;
      try { await refreshRef.current(); } catch { /* The owner keeps its own error state. */ }
      running = false;
      if (disposed) return;
      if (again) { again = false; schedule(0); return; }
      schedule(delayRef.current());
    };
    const now = (): void => {
      if (timer !== undefined) clearTimeout(timer);
      void run();
    };
    const onVisibility = (): void => { if (document.visibilityState === "visible") now(); };
    document.addEventListener("visibilitychange", onVisibility);
    const unsubscribe = subscribe?.(now);
    void run();
    return () => {
      disposed = true;
      if (timer !== undefined) clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
      unsubscribe?.();
    };
  }, [enabled, subscribe]);
}
