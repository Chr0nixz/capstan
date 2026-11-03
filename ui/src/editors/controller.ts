import { createStore } from "zustand/vanilla";
import type { RemoteEditSnapshot, RemoteEditView, SftpErrorCode } from "../sftp/contracts";
import { sftpError } from "../sftp/controller";
import { tauriEditorTransport, type EditorTransport, type EditDecision, type RetryBinding, type EditCloseAction } from "./transport";

type EditorState = { snapshot: RemoteEditSnapshot | null; error: SftpErrorCode | null; pending: string[] };
export function createEditorController(transport: EditorTransport) {
  const store = createStore<EditorState>(() => ({ snapshot: null, error: null, pending: [] }));
  let inFlight: Promise<void> | null = null;
  let generation = 0;
  let disposed = false;
  const merge = (view: RemoteEditView): void => {
    const current = store.getState().snapshot;
    if (!current) return;
    const sessions = current.sessions.filter((session) => session.id !== view.id);
    if (view.state !== "closed") sessions.push(view);
    store.setState({ snapshot: { ...current, sessions } });
  };
  const refresh = (): Promise<void> => {
    if (disposed) return Promise.resolve();
    if (inFlight) return inFlight;
    const operation = generation;
    inFlight = transport.snapshot().then((snapshot) => {
      if (disposed || operation !== generation) return;
      const current = store.getState().snapshot;
      if (current && BigInt(snapshot.revision) < BigInt(current.revision)) return;
      store.setState({ snapshot });
    }).catch((cause: unknown) => {
      if (!disposed && operation === generation) store.setState({ error: sftpError(cause) });
    }).finally(() => { inFlight = null; });
    return inFlight;
  };
  const mutate = async <T,>(id: string, action: () => Promise<T>, apply?: (value: T) => void): Promise<T | undefined> => {
    if (store.getState().pending.includes(id) || disposed) return undefined;
    ++generation;
    store.setState((state) => ({ pending: [...state.pending, id], error: null }));
    try {
      const value = await action();
      if (!disposed) { ++generation; apply?.(value); }
      return value;
    } catch (cause) {
      if (!disposed) store.setState({ error: sftpError(cause) });
      throw cause;
    } finally {
      store.setState((state) => ({ pending: state.pending.filter((entry) => entry !== id) }));
      if (inFlight) await inFlight;
      await refresh();
    }
  };
  return {
    store, refresh,
    start: (): void => { disposed = false; void refresh(); },
    dispose: (): void => { disposed = true; ++generation; },
    opened: (view: RemoteEditView): void => { ++generation; merge(view); },
    retry: (id: string, binding?: RetryBinding) => mutate(id, () => transport.retry(id, binding), merge),
    cleanup: (id: string, binding?: RetryBinding) => mutate(id, () => transport.cleanup(id, binding), merge),
    resolve: (id: string, decision: EditDecision, name?: string) => mutate(id, () => transport.resolve(id, decision, name), merge),
    close: (id: string, action: EditCloseAction, locale: string) => mutate(id, () => transport.close(id, action, locale), (closed) => {
      if (!closed) return;
      const snapshot = store.getState().snapshot;
      if (snapshot) store.setState({ snapshot: { ...snapshot, sessions: snapshot.sessions.filter((session) => session.id !== id) } });
    }),
    discover: () => mutate("discovery", () => transport.discover()),
    configure: (locale: string) => mutate("configuration", () => transport.configure(locale)),
  };
}
export const remoteEditors = createEditorController(tauriEditorTransport());
