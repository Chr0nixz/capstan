import type { Disposable, TerminalLike } from "../terminal/session";

export type TerminalInputOrigin = "user" | "protocol" | "unknown";
type CoreInputService = { triggerDataEvent: (data: string, wasUserInput?: boolean) => void };

function inputService(terminal: unknown): CoreInputService | undefined {
  if (typeof terminal !== "object" || terminal === null || !("_core" in terminal)) return;
  const core = terminal._core;
  if (typeof core !== "object" || core === null || !("coreService" in core)) return;
  const service = core.coreService;
  if (typeof service === "object" && service !== null && "triggerDataEvent" in service && typeof service.triggerDataEvent === "function") {
    return service as CoreInputService;
  }
}

/** xterm 5.4's public onData drops wasUserInput. Preserve that flag at its
 * synchronous dispatcher; never infer origin from focus or response strings.
 * Keep this version-specific adapter isolated and cover it with real xterm. */
export function subscribeTerminalInput(
  terminal: TerminalLike,
  listener: (data: string, origin: TerminalInputOrigin) => void,
): Disposable {
  const service = inputService(terminal);
  let origin: TerminalInputOrigin = "unknown";
  const original = service?.triggerDataEvent;
  const dispatch = function (this: CoreInputService, data: string, wasUserInput = false): void {
    const previous = origin;
    origin = wasUserInput ? "user" : "protocol";
    try { original?.call(this, data, wasUserInput); }
    finally { origin = previous; }
  };
  if (service) {
    try { service.triggerDataEvent = dispatch; }
    catch { /* A read-only or changed private API leaves origin unknown. */ }
  }
  const subscription = terminal.onData((data) => listener(data, origin));
  let disposed = false;
  return { dispose: () => {
    if (disposed) return;
    disposed = true;
    subscription.dispose();
    if (service && original && service.triggerDataEvent === dispatch) service.triggerDataEvent = original;
  } };
}
