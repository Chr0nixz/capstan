import type { Failure } from "./contracts";

export class BenchmarkError extends Error {
  constructor(readonly code: Failure) { super(code); }
}

/** Bounds the awaited IPC itself, including calls that never settle. */
export async function withDeadline<T>(
  operation: (signal: AbortSignal) => Promise<T>, signal: AbortSignal, timeout = 30_000,
): Promise<T> {
  const bounded = new AbortController();
  const cancel = (): void => bounded.abort(new BenchmarkError("cancelled"));
  if (signal.aborted) throw new BenchmarkError("cancelled");
  signal.addEventListener("abort", cancel, { once: true });
  const timer = window.setTimeout(() => bounded.abort(new BenchmarkError("timeout")), timeout);
  let rejectAbort!: () => void;
  const aborted = new Promise<never>((_resolve, reject) => {
    rejectAbort = () => { reject(bounded.signal.reason); };
    bounded.signal.addEventListener("abort", rejectAbort, { once: true });
  });
  try { return await Promise.race([operation(bounded.signal), aborted]); }
  finally {
    window.clearTimeout(timer);
    signal.removeEventListener("abort", cancel);
    bounded.signal.removeEventListener("abort", rejectAbort);
  }
}

export async function until(
  predicate: () => boolean | Promise<boolean>, signal: AbortSignal, timeout = 30_000,
): Promise<void> {
  await withDeadline(async (bounded) => {
    while (!await predicate()) {
      if (bounded.aborted) throw bounded.reason;
      await new Promise<void>((resolve) => window.setTimeout(resolve, 20));
    }
    if (bounded.aborted) throw bounded.reason;
  }, signal, timeout);
}
