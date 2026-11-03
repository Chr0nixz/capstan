import { invokeTauri } from "../terminal/bridge";
import type { TerminalSessionId } from "../terminal/contracts";
import { decodeAttachedSftp, decodeDirectoryPage, decodeRemoteEditView, decodeShellPreview, type AttachedSftp, type Confirmation, type DirectoryPage, type ExactU64, type Mutation, type RemoteEditView, type RemoteEntryId, type Shell, type ShellAction, type ShellPreview, type SftpSessionId } from "./contracts";

export interface SftpTransport {
  attach(): Promise<AttachedSftp>;
  detach(session: SftpSessionId): Promise<void>;
  list(session: SftpSessionId, path: string): Promise<DirectoryPage>;
  next(session: SftpSessionId, revision: ExactU64): Promise<DirectoryPage>;
  release(session: SftpSessionId, entries: RemoteEntryId[]): Promise<void>;
  create(session: SftpSessionId, parent: string, name: string, directory: boolean): Promise<void>;
  prepare(session: SftpSessionId, entry: RemoteEntryId, mutation: Mutation): Promise<Confirmation>;
  apply(session: SftpSessionId, token: string): Promise<void>;
  previewShell?(session: SftpSessionId, entry: RemoteEntryId, shell: Shell, action: ShellAction): Promise<ShellPreview>;
  applyShell?(session: SftpSessionId, entry: RemoteEntryId, token: string): Promise<unknown>;
  selectEditor?(locale: string): Promise<string | null>;
  editorConfiguration?(): Promise<string | null>;
  beginEditor?(session: SftpSessionId, entry: RemoteEntryId, editorId: string): Promise<RemoteEditView>;
}
export function tauriSftpTransport(terminal: TerminalSessionId): SftpTransport {
  return {
    attach: async () => decodeAttachedSftp(await invokeTauri("attach_sftp", { terminal })),
    detach: (session) => invokeTauri("detach_sftp", { terminal, session }),
    list: async (session, path) => decodeDirectoryPage(await invokeTauri("list_remote_directory", { terminal, session, path })),
    next: async (session, revision) => decodeDirectoryPage(await invokeTauri("next_remote_directory_page", { terminal, session, revision })),
    release: (session, entries) => invokeTauri("release_remote_entries", { terminal, session, entries }),
    create: (session, parent, name, directory) => invokeTauri("create_remote_entry", { terminal, session, parent, name, directory }),
    prepare: (session, entry, mutation) => invokeTauri("prepare_remote_mutation", { terminal, session, entry, mutation }),
    apply: (session, token) => invokeTauri("apply_remote_mutation", { terminal, session, token }),
    previewShell: async (session, entry, shell, action) => decodeShellPreview(await invokeTauri("preview_shell_integration", { terminal, session, entry, shell, action })),
    applyShell: (session, entry, token) => invokeTauri("apply_shell_integration", { terminal, session, entry, token }),
    selectEditor: async (locale) => {
      const value = await invokeTauri("select_remote_editor", { locale });
      return typeof value === "string" ? value : null;
    },
    editorConfiguration: () => invokeTauri("remote_editor_configuration"),
    beginEditor: async (session, entry, editorId) => decodeRemoteEditView(await invokeTauri("begin_remote_edit", { terminal, session, entry, editorId })),
  };
}
