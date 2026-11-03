import { invokeTauri } from "../terminal/bridge";
import type { TerminalSessionId } from "../terminal/contracts";
import { sftpErrorCodes, validUtf8, type RemoteEntryId, type SftpErrorCode, type SftpSessionId } from "./contracts";

export type DeleteTreeSummary = { token: string; hostId: string; path: string; files: number; directories: number; symlinks: number };
export type DeleteTreeReport = { entries: Array<{ path: string; outcome: "removed" | "failed" | "retained"; error: SftpErrorCode | null }> };
function invalid(): never { throw { code: "remote_io" }; }
function record(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  const result = value as Record<string, unknown>;
  if (Object.keys(result).length !== keys.length || keys.some((key) => !Object.hasOwn(result, key))) return invalid();
  return result;
}
function uuid(value: unknown): string {
  return typeof value === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value) ? value : invalid();
}
function path(value: unknown): string {
  return typeof value === "string" && value.startsWith("/") && validUtf8(value, 4096) && !/[\u0000-\u001f\u007f-\u009f]/u.test(value) ? value : invalid();
}
export function decodeDeleteTreeSummary(value: unknown): DeleteTreeSummary {
  const data = record(value, ["token", "hostId", "path", "files", "directories", "symlinks"]);
  const counts = [data.files, data.directories, data.symlinks];
  if (counts.some((value) => typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > 4096)) return invalid();
  const files = data.files as number, directories = data.directories as number, symlinks = data.symlinks as number;
  if (directories < 1 || files + directories + symlinks > 4096) return invalid();
  return { token: uuid(data.token), hostId: uuid(data.hostId), path: path(data.path), files, directories, symlinks };
}
export function decodeDeleteTreeReport(value: unknown): DeleteTreeReport {
  const data = record(value, ["entries"]);
  if (!Array.isArray(data.entries) || data.entries.length < 1 || data.entries.length > 4096) return invalid();
  const seen = new Set<string>(); let bytes = 0;
  const entries = data.entries.map((value: unknown): DeleteTreeReport["entries"][number] => {
    const entry = record(value, ["path", "outcome", "error"]), target = path(entry.path);
    bytes += new TextEncoder().encode(target).length;
    if (seen.has(target) || bytes > 256 * 1024 || !["removed", "failed", "retained"].includes(String(entry.outcome))
      || (entry.error !== null && !sftpErrorCodes.includes(entry.error as SftpErrorCode))
      || (entry.outcome === "removed") !== (entry.error === null)) return invalid();
    seen.add(target);
    return { path: target, outcome: entry.outcome as "removed" | "failed" | "retained", error: entry.error as SftpErrorCode | null };
  });
  return { entries };
}
export const prepareTreeDelete = async (terminal: TerminalSessionId, session: SftpSessionId, entry: RemoteEntryId): Promise<DeleteTreeSummary> => decodeDeleteTreeSummary(await invokeTauri("prepare_remote_tree_delete", { terminal, session, entry }));
export const applyTreeDelete = async (terminal: TerminalSessionId, session: SftpSessionId, token: string): Promise<DeleteTreeReport> => decodeDeleteTreeReport(await invokeTauri("apply_remote_tree_delete", { terminal, session, token }));
export const cancelTreeDelete = (terminal: TerminalSessionId, session: SftpSessionId, token: string): Promise<void> => invokeTauri("cancel_remote_tree_delete", { terminal, session, token });
