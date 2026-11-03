// This decoder is a metadata boundary. File contents never enter this contract.
export type ExactU64 = string & { readonly exactU64: unique symbol };
export type SftpSessionId = string & { readonly sftpSession: unique symbol };
export type RemoteEntryId = string & { readonly remoteEntry: unique symbol };
export type FileKind = "file" | "directory" | "symlink" | "other" | "unknown";
export type Metadata = {
  kind: FileKind;
  size: ExactU64 | null;
  modifiedSeconds: ExactU64 | null;
  permissions: number | null;
};
export type DirectoryEntry = { id: RemoteEntryId; name: string; metadata: Metadata };
export type DirectoryPage = {
  sessionId: SftpSessionId;
  requestRevision: ExactU64;
  pageSequence: number;
  path: string;
  entries: DirectoryEntry[];
  hasMore: boolean;
  omittedEntries: number;
};
export type AttachedSftp = {
  sessionId: SftpSessionId; hostId: string; connectionGeneration: ExactU64;
  path: string; atomicReplace: boolean; fsync: boolean; requestBytes: number;
};
export type Mutation = { kind: "delete" } | { kind: "rename"; name: string } | { kind: "permissions"; mode: number };
export type Shell = "bash" | "zsh";
export type ShellAction = "install" | "uninstall";
export type ShellPreview = {
  token: string; shell: Shell; action: ShellAction; host: string;
  sourceHash: string; replacementHash: string; addedLines: number; removedLines: number;
};
export type RemoteEditState = "opening" | "waiting_for_stable_save" | "ready_to_upload" | "conflict" | "residual" | "closed";
export type RemoteTemporaryState = "unverified" | "recorded" | "changed";
export type RemoteTemporaryView = { id: string; path: string; state: RemoteTemporaryState };
export type RemoteEditView = {
  id: string; name: string; remotePath: string | null; hostId: string | null; state: RemoteEditState;
  dirty: boolean; recovered: boolean; cleanupRequired: boolean; replaceAllowed: boolean; error: SftpErrorCode | null;
  remoteTemporaries: RemoteTemporaryView[];
};
export type RemoteEditSnapshot = { revision: ExactU64; sessions: RemoteEditView[]; recoveryIncomplete: boolean };
export type Confirmation = { token: string; hostId: string; path: string; metadata: Metadata; mutation: Mutation };
export const sftpErrorCodes = [
  "sftp_unavailable", "permission_denied", "path_invalid", "not_found", "unsupported_file_type",
  "target_conflict", "resume_mismatch", "local_io", "remote_io", "editor_unavailable",
  "connection_lost", "operation_canceled", "scope_expired", "resource_limit",
  "editor_file_too_large", "editor_invalid_encoding", "editor_unsupported_content", "editor_conflict", "editor_stale_save", "editor_residual",
] as const;
export type SftpErrorCode = typeof sftpErrorCodes[number];

const maximumU64 = "18446744073709551615";
const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const controlPattern = /[\u0000-\u001f\u007f-\u009f]/u;
const invalidContract = (): never => { throw new Error("sftp_contract_invalid"); };

export function validUtf8(value: string, maximumBytes: number): boolean {
  const bytes = new TextEncoder().encode(value);
  // TextEncoder silently replaces lone UTF-16 surrogates. Rust's JSON parser
  // rejects them, so reject that lossy conversion at this boundary too.
  return bytes.length <= maximumBytes && new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes) === value;
}

export function decodeExactU64(value: unknown): ExactU64 {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]*)$/.test(value)
    || value.length > maximumU64.length || (value.length === maximumU64.length && value > maximumU64)) {
    return invalidContract();
  }
  return value as ExactU64;
}

function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalidContract();
  const result = value as Record<string, unknown>;
  if (Object.keys(result).length !== keys.length || !keys.every((key) => Object.hasOwn(result, key))) return invalidContract();
  return result;
}

function uuid(value: unknown): string {
  if (typeof value !== "string" || !uuidPattern.test(value)) return invalidContract();
  return value;
}

function nullableU64(value: unknown): ExactU64 | null {
  return value === null ? null : decodeExactU64(value);
}

export function decodeMetadata(value: unknown): Metadata {
  const data = record(value, ["kind", "size", "modifiedSeconds", "permissions"]);
  const kind = data.kind;
  if (kind !== "file" && kind !== "directory" && kind !== "symlink" && kind !== "other" && kind !== "unknown") return invalidContract();
  const permissions = data.permissions;
  if (permissions !== null && (typeof permissions !== "number" || !Number.isInteger(permissions) || permissions < 0 || permissions > 0xffff_ffff)) return invalidContract();
  return { kind, size: nullableU64(data.size), modifiedSeconds: nullableU64(data.modifiedSeconds), permissions };
}

export function decodeDirectoryPage(value: unknown): DirectoryPage {
  const data = record(value, ["sessionId", "requestRevision", "pageSequence", "path", "entries", "hasMore", "omittedEntries"]);
  const path = data.path;
  if (typeof path !== "string" || !path.startsWith("/") || !validUtf8(path, 4096) || controlPattern.test(path)) return invalidContract();
  if (!Array.isArray(data.entries) || data.entries.length > 256 || typeof data.hasMore !== "boolean") return invalidContract();
  const pageSequence = data.pageSequence;
  const omittedEntries = data.omittedEntries;
  if (typeof omittedEntries !== "number" || !Number.isInteger(omittedEntries) || omittedEntries < 0 || omittedEntries > 0xffff_ffff) return invalidContract();
  if (typeof pageSequence !== "number" || !Number.isInteger(pageSequence) || pageSequence < 0 || pageSequence > 0xffff_ffff) return invalidContract();
  const seen = new Set<string>();
  const entries = data.entries.map((value: unknown): DirectoryEntry => {
    const entry = record(value, ["id", "name", "metadata"]);
    const id = uuid(entry.id) as RemoteEntryId;
    const name = entry.name;
    if (seen.has(id) || typeof name !== "string" || name.length === 0 || name === "." || name === ".."
      || name.includes("/") || name.includes("\ufffd") || controlPattern.test(name) || !validUtf8(name, 255)) return invalidContract();
    seen.add(id);
    return { id, name, metadata: decodeMetadata(entry.metadata) };
  });
  return {
    sessionId: uuid(data.sessionId) as SftpSessionId,
    requestRevision: decodeExactU64(data.requestRevision), pageSequence, path, entries, hasMore: data.hasMore, omittedEntries,
  };
}

export function decodeAttachedSftp(value: unknown): AttachedSftp {
  const data = record(value, ["sessionId", "hostId", "connectionGeneration", "path", "atomicReplace", "fsync", "requestBytes"]);
  if (typeof data.path !== "string" || !data.path.startsWith("/") || !validUtf8(data.path, 4096) || controlPattern.test(data.path)
    || typeof data.atomicReplace !== "boolean" || typeof data.fsync !== "boolean"
    || typeof data.requestBytes !== "number" || !Number.isInteger(data.requestBytes) || data.requestBytes < 1 || data.requestBytes > 32768) return invalidContract();
  return { sessionId: uuid(data.sessionId) as SftpSessionId, hostId: uuid(data.hostId), connectionGeneration: decodeExactU64(data.connectionGeneration),
    path: data.path, atomicReplace: data.atomicReplace, fsync: data.fsync, requestBytes: data.requestBytes };
}

export function decodeShellPreview(value: unknown): ShellPreview {
  const data = record(value, ["token", "shell", "action", "host", "sourceHash", "replacementHash", "addedLines", "removedLines"]);
  if (typeof data.token !== "string" || !uuidPattern.test(data.token)
    || (data.shell !== "bash" && data.shell !== "zsh")
    || (data.action !== "install" && data.action !== "uninstall")
    || typeof data.host !== "string" || !validUtf8(data.host, 255) || controlPattern.test(data.host)
    || typeof data.sourceHash !== "string" || !/^[0-9A-F]{64}$/u.test(data.sourceHash)
    || typeof data.replacementHash !== "string" || !/^[0-9A-F]{64}$/u.test(data.replacementHash)
    || typeof data.addedLines !== "number" || !Number.isInteger(data.addedLines) || data.addedLines < 0 || data.addedLines > 65536
    || typeof data.removedLines !== "number" || !Number.isInteger(data.removedLines) || data.removedLines < 0 || data.removedLines > 65536) return invalidContract();
  return {
    token: data.token, shell: data.shell, action: data.action, host: data.host,
    sourceHash: data.sourceHash, replacementHash: data.replacementHash,
    addedLines: data.addedLines, removedLines: data.removedLines,
  };
}

export function decodeRemoteEditView(value: unknown): RemoteEditView {
  const data = record(value, ["id", "name", "remotePath", "hostId", "state", "dirty", "recovered", "cleanupRequired", "replaceAllowed", "error", "remoteTemporaries"]);
  const id = uuid(data.id);
  const remotePath = data.remotePath;
  const states: RemoteEditState[] = ["opening", "waiting_for_stable_save", "ready_to_upload", "conflict", "residual", "closed"];
  if (remotePath !== null && (typeof remotePath !== "string" || !remotePath.startsWith("/") || !validUtf8(remotePath, 4096) || controlPattern.test(remotePath))
    || typeof data.name !== "string" || !data.name || !validUtf8(data.name, 1024) || controlPattern.test(data.name)
    || data.name.includes("/") || data.name.includes("\\") || data.name === "." || data.name === ".."
    || !states.includes(data.state as RemoteEditState) || typeof data.cleanupRequired !== "boolean"
    || typeof data.dirty !== "boolean" || typeof data.recovered !== "boolean" || typeof data.replaceAllowed !== "boolean"
    || data.error !== null && !sftpErrorCodes.includes(data.error as SftpErrorCode)) return invalidContract();
  if (!Array.isArray(data.remoteTemporaries) || data.remoteTemporaries.length > 8) return invalidContract();
  const remoteTemporaries = data.remoteTemporaries.map((value: unknown): RemoteTemporaryView => {
    const temporary = record(value, ["id", "path", "state"]);
    if (typeof temporary.path !== "string" || !temporary.path.startsWith("/") || !validUtf8(temporary.path, 4096) || controlPattern.test(temporary.path)
      || temporary.state !== "unverified" && temporary.state !== "recorded" && temporary.state !== "changed") return invalidContract();
    return { id: uuid(temporary.id), path: temporary.path, state: temporary.state };
  });
  if (new Set(remoteTemporaries.map((temporary) => temporary.id)).size !== remoteTemporaries.length) return invalidContract();
  return { id, name: data.name, remotePath, hostId: data.hostId === null ? null : uuid(data.hostId), state: data.state as RemoteEditState,
    dirty: data.dirty, recovered: data.recovered, cleanupRequired: data.cleanupRequired, replaceAllowed: data.replaceAllowed, remoteTemporaries, error: data.error as SftpErrorCode | null };
}

export function decodeRemoteEditSnapshot(value: unknown): RemoteEditSnapshot {
  const data = record(value, ["revision", "sessions", "recoveryIncomplete"]);
  if (!Array.isArray(data.sessions) || data.sessions.length > 8 || typeof data.recoveryIncomplete !== "boolean") return invalidContract();
  const sessions = data.sessions.map(decodeRemoteEditView);
  if (new Set(sessions.map((session) => session.id)).size !== sessions.length) return invalidContract();
  return { revision: decodeExactU64(data.revision), sessions, recoveryIncomplete: data.recoveryIncomplete };
}
