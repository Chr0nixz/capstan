import { createStore } from "zustand/vanilla";
import { sftpError } from "../sftp/controller";
import type { SftpErrorCode } from "../sftp/contracts";
import { isFinal, type Decision, type Transfer, type TransferRecovery } from "./contracts";
import type { TransferTransport } from "./transport";
import type { SftpSessionId } from "../sftp/contracts";
import type { TerminalSessionId } from "../terminal/contracts";
type QueueState = { tasks: Transfer[]; recovery: TransferRecovery[]; recoveryIncomplete: boolean; error: SftpErrorCode | null; pending: string[] };
export function createTransferController(transport: TransferTransport) {
  const store = createStore<QueueState>(() => ({ tasks: [], recovery: [], recoveryIncomplete: false, error: null, pending: [] }));
  let generation = 0, refreshing = false, recoveryReadAt = Number.NEGATIVE_INFINITY;
  const refresh = async (): Promise<void> => {
    if (refreshing) return;
    const operation = generation;
    refreshing = true;
    try {
      const recoveryRead = (): Promise<{ entries: TransferRecovery[]; incomplete: boolean }> => {
        const now = Date.now();
        if (!transport.recoverySnapshot) return Promise.resolve({ entries: [], incomplete: false });
        if (now - recoveryReadAt < 2_000) {
          const current = store.getState();
          return Promise.resolve({ entries: current.recovery, incomplete: current.recoveryIncomplete });
        }
        recoveryReadAt = now;
        return transport.recoverySnapshot();
      };
      const [snapshot, recovery] = await Promise.all([transport.snapshot(), recoveryRead()]);
      if (operation !== generation) return;
      const previous = new Map(store.getState().tasks.map((task) => [task.id, task]));
      const next = new Map(snapshot.map((task) => {
        const old = previous.get(task.id);
        // A delayed/stale progress row can never replace a final result.
        return [task.id, old && (BigInt(old.version) >= BigInt(task.version) || isFinal(old.state)) ? old : task];
      }));
      const order = [...previous.keys(), ...next.keys()].filter((id, index, values) => next.has(id) && values.indexOf(id) === index);
      store.setState({ tasks: order.map((id) => next.get(id)).filter((task): task is Transfer => task !== undefined), recovery: recovery.entries.filter((entry) => {
        const task = next.get(entry.id);
        return !task || (isFinal(task.state) && (entry.state === "cleanup_required" || entry.state === "manual_inspection"));
      }), recoveryIncomplete: recovery.incomplete });
    } catch (cause) { if (operation === generation) store.setState({ error: sftpError(cause) }); }
    finally { refreshing = false; }
  };
  const action = async (id: string, run: () => Promise<void>, rethrow = false, retry = false): Promise<void> => {
    if (store.getState().pending.includes(id)) return;
    const operation = generation;
    store.setState({ pending: [...store.getState().pending, id], error: null });
    try {
      await run();
      if (operation === generation) recoveryReadAt = Number.NEGATIVE_INFINITY;
      if (retry && operation === generation) {
        // Only an explicit successful retry reopens a final row. Keep its
        // revision so delayed snapshots cannot undo the accepted command.
        store.setState({ tasks: store.getState().tasks.map((task) => task.id === id && isFinal(task.state)
          ? { ...task, state: "retrying", error: null, conflict: null, cleanupRequired: false, processed: "0" as Transfer["processed"] }
          : task) });
      }
    }
    catch (cause) {
      if (operation === generation) store.setState({ error: sftpError(cause) });
      if (rethrow) throw cause;
    }
    finally {
      if (operation === generation) { store.setState({ pending: store.getState().pending.filter((value) => value !== id) }); await refresh(); }
    }
  };
  return {
    store, refresh,
    cancel: (id: string): Promise<void> => action(id, () => transport.cancel(id)),
    pause: (id: string): Promise<void> => action(id, () => transport.pause?.(id) ?? Promise.reject({ code: "operation_canceled" })),
    resume: (id: string): Promise<void> => action(id, () => transport.resume?.(id) ?? Promise.reject({ code: "operation_canceled" })),
    retry: (id: string): Promise<void> => action(id, () => transport.retry?.(id) ?? Promise.reject({ code: "operation_canceled" }), false, true),
    retryOnSession: (id: string, terminal: TerminalSessionId, session: SftpSessionId): Promise<void> =>
      action(id, () => transport.retryOnSession?.(id, terminal, session) ?? Promise.reject({ code: "operation_canceled" }), true, true),
    retryWithSelection: (id: string, terminal: TerminalSessionId, session: SftpSessionId, selection: string): Promise<void> =>
      action(id, () => transport.retryWithSelection?.(id, terminal, session, selection) ?? Promise.reject({ code: "operation_canceled" }), true, true),
    retryRecoveredUpload: (id: string, terminal: TerminalSessionId, session: SftpSessionId, selection: string): Promise<void> =>
      action(id, () => transport.retryRecoveredUpload?.(id, terminal, session, selection) ?? Promise.reject({ code: "operation_canceled" }), true, true),
    retryRecoveredDownload: (id: string, terminal: TerminalSessionId, session: SftpSessionId, selection: string): Promise<void> =>
      action(id, () => transport.retryRecoveredDownload?.(id, terminal, session, selection) ?? Promise.reject({ code: "operation_canceled" }), true, true),
    cleanupRecoveredDownload: (id: string, selection: string): Promise<void> =>
      action(id, () => transport.cleanupRecoveredDownload?.(id, selection) ?? Promise.reject({ code: "operation_canceled" }), true),
    cleanupRecoveredUpload: (id: string, terminal: TerminalSessionId, session: SftpSessionId): Promise<void> =>
      action(id, () => transport.cleanupRecoveredUpload?.(id, terminal, session) ?? Promise.reject({ code: "operation_canceled" }), true),
    resolve: (task: Transfer, decision: Decision): Promise<void> => {
      const conflict = task.conflict;
      return conflict ? action(task.id, () => transport.resolve(task.id, conflict.revision, decision)) : Promise.resolve();
    },
    exit: (): Promise<void> => transport.exit(),
    dispose: (): void => { generation++; },
  };
}
