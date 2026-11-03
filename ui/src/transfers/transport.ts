import { invokeTauri } from "../terminal/bridge";
import { decodeTransferRecovery, decodeTransfers, type Decision, type Transfer, type TransferRecoverySnapshot } from "./contracts";
import type { ExactU64 } from "../sftp/contracts";
import type { SftpSessionId } from "../sftp/contracts";
import type { TerminalSessionId } from "../terminal/contracts";
export interface TransferTransport {
  snapshot(): Promise<Transfer[]>;
  recoverySnapshot?(): Promise<TransferRecoverySnapshot>;
  cancel(id: string): Promise<void>;
  pause?(id: string): Promise<void>;
  resume?(id: string): Promise<void>;
  retry?(id: string): Promise<void>;
  retryOnSession?(id: string, terminal: TerminalSessionId, session: SftpSessionId): Promise<void>;
  retryWithSelection?(id: string, terminal: TerminalSessionId, session: SftpSessionId, selection: string): Promise<void>;
  retryRecoveredUpload?(id: string, terminal: TerminalSessionId, session: SftpSessionId, selection: string): Promise<void>;
  retryRecoveredDownload?(id: string, terminal: TerminalSessionId, session: SftpSessionId, selection: string): Promise<void>;
  cleanupRecoveredDownload?(id: string, selection: string): Promise<void>;
  cleanupRecoveredUpload?(id: string, terminal: TerminalSessionId, session: SftpSessionId): Promise<void>;
  resolve(id: string, revision: ExactU64, decision: Decision): Promise<void>;
  exit(): Promise<void>;
}
export function tauriTransferTransport(): TransferTransport {
  return {
    snapshot: async () => decodeTransfers(await invokeTauri("transfer_snapshot")),
    recoverySnapshot: async () => decodeTransferRecovery((await invokeTauri("transfer_recovery_snapshot")) ?? { entries: [], incomplete: false }),
    cancel: (id) => invokeTauri("cancel_transfer", { id }),
    pause: (id) => invokeTauri("pause_transfer", { id }),
    resume: (id) => invokeTauri("resume_transfer", { id }),
    retry: (id) => invokeTauri("retry_transfer", { id }),
    retryOnSession: (id, terminal, session) => invokeTauri("retry_transfer_on_session", { id, terminal, session }),
    retryWithSelection: (id, terminal, session, selection) => invokeTauri("retry_transfer_with_selection", { id, terminal, session, selection }),
    retryRecoveredUpload: (id, terminal, session, selection) => invokeTauri("retry_recovered_transfer_upload", { id, terminal, session, selection }),
    retryRecoveredDownload: (id, terminal, session, selection) => invokeTauri("retry_recovered_transfer_download", { id, terminal, session, selection }),
    cleanupRecoveredDownload: (id, selection) => invokeTauri("cleanup_recovered_transfer_download", { id, selection }),
    cleanupRecoveredUpload: (id, terminal, session) => invokeTauri("cleanup_recovered_transfer_upload", { id, terminal, session }),
    resolve: (id, revision, decision) => invokeTauri("resolve_transfer_conflict", { id, revision, decision }),
    exit: () => invokeTauri("confirm_transfer_exit"),
  };
}
