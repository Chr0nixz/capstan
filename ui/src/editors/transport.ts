import { invokeTauri } from "../terminal/bridge";
import { decodeRemoteEditSnapshot, decodeRemoteEditView, type RemoteEditSnapshot, type RemoteEditView } from "../sftp/contracts";
import type { SftpSessionId } from "../sftp/contracts";
import type { TerminalSessionId } from "../terminal/contracts";
export type RetryBinding = { terminal: TerminalSessionId; session: SftpSessionId; hostId: string };

export type EditDecision = "keep_remote" | "replace_remote" | "keep_local_as_new";
export type EditCloseAction = "save_local_copy" | "discard" | "keep_for_recovery";
export interface EditorTransport {
  snapshot(): Promise<RemoteEditSnapshot>;
  discover(): Promise<void>;
  configure(locale: string): Promise<string | null>;
  retry(id: string, binding?: RetryBinding): Promise<RemoteEditView>;
  cleanup(id: string, binding?: RetryBinding): Promise<RemoteEditView>;
  resolve(id: string, decision: EditDecision, newName?: string): Promise<RemoteEditView>;
  close(id: string, action: EditCloseAction, locale: string): Promise<boolean>;
}
export function tauriEditorTransport(): EditorTransport {
  return {
    snapshot: async () => decodeRemoteEditSnapshot(await invokeTauri("remote_edit_snapshot")),
    discover: () => invokeTauri("discover_remote_edit_residuals"),
    configure: (locale) => invokeTauri("select_remote_editor", { locale }),
    retry: async (id, binding) => decodeRemoteEditView(await invokeTauri("retry_remote_edit", { id, terminal: binding?.terminal ?? null, session: binding?.session ?? null })),
    cleanup: async (id, binding) => decodeRemoteEditView(await invokeTauri("cleanup_remote_edit_temporaries", { id, terminal: binding?.terminal ?? null, session: binding?.session ?? null })),
    resolve: async (id, decision, newName) => decodeRemoteEditView(await invokeTauri("resolve_remote_edit_conflict", { id, decision, newName: newName ?? null })),
    close: (id, action, locale) => invokeTauri("close_remote_edit", { id, action, locale }),
  };
}
