import { InputQueue } from "../terminal/inputQueue";
import type { BroadcastConfirmation, BroadcastRoute, PaneId } from "./contracts";
import { confirmInputRoute } from "./contracts";

export type InputDeliveryResult = {
  deliveredPaneIds: PaneId[];
  failedTargets: { paneId: PaneId; errorCode: string }[];
};
type Pending = { resolve: () => void; reject: (error: unknown) => void };
type Endpoint = { send: (bytes: Uint8Array) => Promise<void>; close: () => void };
export type ConfirmBroadcast = (targets: readonly PaneId[], reason: BroadcastConfirmation) => Promise<boolean>;
export type PreparedInput = (
  bytes: Uint8Array,
  confirm: ConfirmBroadcast,
  isCurrent?: () => boolean,
) => Promise<InputDeliveryResult>;
const failure = (code: string): { code: string } => ({ code });

/** Only stable command codes cross into UI results, never exception/server text. */
export function deliveryErrorCode(error: unknown): string {
  if (error instanceof Error && error.message === "session_closed") return "session_closed";
  const code = typeof error === "object" && error !== null && "code" in error ? error.code : undefined;
  return typeof code === "string" && ["session_closed", "queue_full", "chunk_too_large", "input_queue_full", "not_found", "invalid_input", "io", "backend", "cancelled", "task_failed"].includes(code)
    ? code : "delivery_unknown";
}

/** One bounded FIFO per destination, shared by direct and broadcast producers. */
function endpoint(write: (bytes: Uint8Array) => Promise<void>): Endpoint {
  const queue = new InputQueue(), pending: Pending[] = [];
  let sending = false, closed = false;
  let activeRequest: Pending | undefined;
  const close = (): void => {
    if (closed) return;
    closed = true;
    queue.close();
    pending.splice(0).forEach((request) => request.reject(failure(request === activeRequest ? "delivery_unknown" : "session_closed")));
  };
  const pump = async (): Promise<void> => {
    if (sending || closed) return;
    sending = true;
    try {
      while (!closed) {
        const bytes = queue.peek(), request = pending[0];
        if (!bytes || !request) break;
        activeRequest = request;
        try {
          await write(bytes);
          request.resolve();
        } catch (error) {
          request.reject(failure(deliveryErrorCode(error)));
          // A failed command may already have reached the shell. Discard all
          // later input to this destination; reconnecting never replays it.
          if (!closed) {
            pending.shift();
            queue.dequeue();
            close();
          }
          break;
        }
        if (!closed) { pending.shift(); queue.dequeue(); }
      }
    } finally { activeRequest = undefined; sending = false; }
  };
  return {
    send: (bytes) => new Promise<void>((resolve, reject) => {
      if (closed) { reject(failure("session_closed")); return; }
      if (!queue.enqueue(bytes).accepted) { reject(failure("input_queue_full")); return; }
      pending.push({ resolve, reject });
      void pump();
    }),
    close,
  };
}

export function createPaneInputs() {
  const endpoints = new Map<PaneId, Endpoint>();
  const prepare = (route: BroadcastRoute): PreparedInput => {
    // Freeze IDs, policy and endpoint owners at the source queue boundary.
    // A later selection change or restart cannot redirect already queued input.
    const frozen = { ...route, targetPaneIds: [...new Set(route.targetPaneIds)] };
    const targets = frozen.targetPaneIds.map((paneId) => ({ paneId, owner: endpoints.get(paneId) }));
    return async (bytes, confirm, isCurrent = () => true) => {
      const input = bytes.slice();
      const accepted = !frozen.requiresConfirmation || await confirm(targets.map((target) => target.paneId), frozen.confirmation ?? "multiline");
      const result: InputDeliveryResult = { deliveredPaneIds: [], failedTargets: [] };
      if (!isCurrent() || confirmInputRoute(frozen, accepted).length === 0) return result;
      const outcomes = await Promise.all(targets.map(async ({ paneId, owner }) => {
        try {
          if (!owner) throw failure("session_closed");
          await owner.send(input);
          return { paneId, errorCode: null };
        } catch (error) { return { paneId, errorCode: deliveryErrorCode(error) }; }
      }));
      for (const outcome of outcomes) {
        if (outcome.errorCode === null) result.deliveredPaneIds.push(outcome.paneId);
        else result.failedTargets.push({ paneId: outcome.paneId, errorCode: outcome.errorCode });
      }
      return result;
    };
  };
  return {
    register(id: PaneId, write: (bytes: Uint8Array) => Promise<void>): () => void {
      endpoints.get(id)?.close();
      const owner = endpoint(write);
      endpoints.set(id, owner);
      return () => {
        owner.close();
        if (endpoints.get(id) === owner) endpoints.delete(id);
      };
    },
    prepare,
    dispatch(
      route: BroadcastRoute,
      bytes: Uint8Array,
      confirm: ConfirmBroadcast,
      isCurrent: () => boolean = () => true,
    ): Promise<InputDeliveryResult> {
      return prepare(route)(bytes, confirm, isCurrent);
    },
  };
}

export const paneInputs = createPaneInputs();
