import { decodeExactU64, decodeMetadata, sftpErrorCodes, type ExactU64, type Metadata, type SftpErrorCode } from "../sftp/contracts";
export const transferStates = ["queued", "preparing", "running", "paused", "resuming", "retrying", "verifying", "committing", "completed", "awaiting_conflict", "failed", "canceled", "interrupted"] as const;
export type TransferState = typeof transferStates[number];
export type Decision = { kind: "replace" | "replace_batch" } | { kind: "rename"; name: string } | { kind: "skip" | "skip_batch" };
export type Selection = { token: string; name: string };
export type Transfer = {
  id: string; hostId: string; name: string; remotePath: string; direction: "upload" | "download";
  state: TransferState; version: ExactU64; processed: ExactU64; total: ExactU64;
  bytesPerSecond: ExactU64; remainingSeconds: ExactU64 | null; error: SftpErrorCode | null;
  conflict: { revision: ExactU64; target: Metadata | null; replaceAllowed: boolean; batchEligible: boolean } | null; cleanupRequired: boolean;
  batch: { id: string; name: string; files: number; conflicts: number } | null;
};
export type TransferRecovery = {
  id: string; hostId: string; name: string; remotePath: string; direction: "upload" | "download";
  total: ExactU64; partialSize: ExactU64 | null; localPartial: boolean;
  state: "awaiting_reconnect" | "available" | "missing" | "changed" | "manual_inspection" | "cleanup_required"; updatedSeconds: ExactU64;
};
export type TransferRecoverySnapshot = { entries: TransferRecovery[]; incomplete: boolean };
export function isFinal(state: TransferState): boolean { return ["completed", "failed", "canceled", "interrupted"].includes(state); }
function invalid(): never { throw { code: "remote_io" }; }
function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  const result = value as Record<string, unknown>;
  if (Object.keys(result).length !== keys.length || keys.some((key) => !Object.hasOwn(result, key))) return invalid();
  return result;
}
function uuid(value: unknown): string {
  if (typeof value !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)) return invalid();
  return value;
}
function text(value: unknown, maximum: number): string {
  if (typeof value !== "string" || value.length === 0 || /[\u0000-\u001f\u007f-\u009f]/u.test(value)
    || new TextEncoder().encode(value).length > maximum) return invalid();
  return value;
}
export function decodeSelection(value: unknown): Selection | null {
  if (value === null) return null;
  const data = record(value, ["token", "name"]);
  return { token: uuid(data.token), name: text(data.name, 1024) };
}
export function decodeTransfers(value: unknown): Transfer[] {
  if (!Array.isArray(value) || value.length > 1024) return invalid();
  const ids = new Set<string>();
  return value.map((item: unknown) => {
    const data = record(item, ["id", "hostId", "name", "remotePath", "direction", "state", "version", "processed", "total", "bytesPerSecond", "remainingSeconds", "error", "conflict", "cleanupRequired", "batch"]);
    const id = uuid(data.id);
    if (ids.has(id)) return invalid();
    ids.add(id);
    const state = transferStates.find((state) => state === data.state);
    const error = data.error === null ? null : sftpErrorCodes.find((code) => code === data.error);
    const path = text(data.remotePath, 4096);
    if (!state || error === undefined || !path.startsWith("/") || typeof data.cleanupRequired !== "boolean"
      || (data.direction !== "upload" && data.direction !== "download")) return invalid();
    const processed = decodeExactU64(data.processed), total = decodeExactU64(data.total);
    if (BigInt(processed) > BigInt(total)) return invalid();
    let batch: Transfer["batch"] = null;
    if (data.batch !== null) {
      const group = record(data.batch, ["id", "name", "files", "conflicts"]);
      if (typeof group.files !== "number" || !Number.isInteger(group.files) || group.files < 1 || group.files > 1024
        || typeof group.conflicts !== "number" || !Number.isInteger(group.conflicts) || group.conflicts < 0 || group.conflicts > group.files) return invalid();
      batch = { id: uuid(group.id), name: text(group.name, 1024), files: group.files, conflicts: group.conflicts };
    }
    let conflict: Transfer["conflict"] = null;
    if (data.conflict !== null) {
      const prompt = record(data.conflict, ["revision", "target", "replaceAllowed", "batchEligible"]);
      if (state !== "awaiting_conflict" || typeof prompt.replaceAllowed !== "boolean" || typeof prompt.batchEligible !== "boolean"
        || (prompt.batchEligible && (!batch || batch.conflicts === 0 || prompt.target === null))) return invalid();
      conflict = { revision: decodeExactU64(prompt.revision), target: prompt.target === null ? null : decodeMetadata(prompt.target), replaceAllowed: prompt.replaceAllowed, batchEligible: prompt.batchEligible };
    }
    return { id, hostId: uuid(data.hostId), name: text(data.name, 1024), remotePath: path, direction: data.direction, state,
      version: decodeExactU64(data.version), processed, total, bytesPerSecond: decodeExactU64(data.bytesPerSecond),
      remainingSeconds: data.remainingSeconds === null ? null : decodeExactU64(data.remainingSeconds), error, conflict, cleanupRequired: data.cleanupRequired, batch };
  });
}
export function decodeTransferRecovery(value: unknown): TransferRecoverySnapshot {
  const snapshot = record(value, ["entries", "incomplete"]);
  if (!Array.isArray(snapshot.entries) || snapshot.entries.length > 128 || typeof snapshot.incomplete !== "boolean") return invalid();
  const ids = new Set<string>();
  const entries = snapshot.entries.map((item: unknown): TransferRecovery => {
    const data = record(item, ["id", "hostId", "name", "remotePath", "direction", "total", "partialSize", "localPartial", "state", "updatedSeconds"]);
    const id = uuid(data.id), hostId = uuid(data.hostId);
    const path = text(data.remotePath, 4096);
    const states = ["awaiting_reconnect", "available", "missing", "changed", "manual_inspection", "cleanup_required"] as const;
    if (ids.has(id) || !path.startsWith("/") || (data.direction !== "upload" && data.direction !== "download")
      || typeof data.localPartial !== "boolean" || !states.includes(data.state as typeof states[number])) return invalid();
    ids.add(id);
    return { id, hostId, name: text(data.name, 1024), remotePath: path, direction: data.direction,
      total: decodeExactU64(data.total), partialSize: data.partialSize === null ? null : decodeExactU64(data.partialSize),
      localPartial: data.localPartial, state: data.state as TransferRecovery["state"], updatedSeconds: decodeExactU64(data.updatedSeconds) };
  });
  return { entries, incomplete: snapshot.incomplete };
}
