import type { TransferState } from "./contracts";

/** While bytes are moving, the queue refreshes four times a second. */
export const ACTIVE_POLL_MS = 250;
/** With nothing moving, a slow check still finds work started elsewhere. */
export const IDLE_POLL_MS = 2_000;

const listeners = new Set<() => void>();

/** Asks the transfer and export queues to refresh now, e.g. right after work is queued. */
export function notifyTransfersChanged(): void {
  for (const listener of [...listeners]) listener();
}

export function onTransfersChanged(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

// A conflict on screen stays fresh too, so batch consent never outlives the
// target revision it was given for.
const live: readonly TransferState[] = ["queued", "preparing", "running", "resuming", "retrying", "verifying", "committing", "awaiting_conflict"];

/** Paused and finished rows change only when the user acts, so they poll at the idle rate. */
export function transferPollDelay(tasks: readonly { state: TransferState }[], pending: readonly string[]): number {
  return pending.length > 0 || tasks.some((task) => live.includes(task.state)) ? ACTIVE_POLL_MS : IDLE_POLL_MS;
}
