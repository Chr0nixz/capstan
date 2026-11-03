import { invokeTauri } from "../terminal/bridge";
import { decodeExactU64, type ExactU64, type RemoteEntryId, type SftpSessionId } from "../sftp/contracts";
import type { TerminalSessionId } from "../terminal/contracts";
export type TreeConflict = { entry: number; source: string; relative: string; directory: boolean; kind: "directory_exists" | "type_conflict" | "invalid_name" | "name_collision" };
export type TreePlan = { token: string; revision: ExactU64; name: string; files: number; conflicts: TreeConflict[] };
export type TreeDecision = { entry: number; decision: { kind: "merge" | "skip" } | { kind: "rename"; name: string } };
function invalid(): never { throw { code: "remote_io" }; }
function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  const data = value as Record<string, unknown>;
  if (Object.keys(data).length !== keys.length || keys.some((key) => !Object.hasOwn(data, key))) return invalid();
  return data;
}
function text(value: unknown): string { if (typeof value !== "string" || !value || value.length > 4096 || /[\u0000-\u001f\u007f]/u.test(value)) return invalid(); return value; }
export function decodeTreePlan(value: unknown): TreePlan | null {
  if (value === null) return null;
  const data = record(value, ["token", "revision", "name", "files", "conflicts"]);
  if (typeof data.token !== "string" || !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu.test(data.token)
    || typeof data.files !== "number" || !Number.isInteger(data.files) || data.files < 0 || data.files > 1024 || !Array.isArray(data.conflicts) || data.conflicts.length > 4096) return invalid();
  const ids = new Set<number>();
  const conflicts = data.conflicts.map((value): TreeConflict => {
    const conflict = record(value, ["entry", "source", "relative", "directory", "kind"]);
    if (typeof conflict.entry !== "number" || !Number.isInteger(conflict.entry) || conflict.entry < 0 || conflict.entry >= 4096 || ids.has(conflict.entry)
      || typeof conflict.directory !== "boolean" || !["directory_exists", "type_conflict", "invalid_name", "name_collision"].includes(String(conflict.kind))) return invalid();
    ids.add(conflict.entry);
    return { entry: conflict.entry, source: text(conflict.source), relative: text(conflict.relative), directory: conflict.directory, kind: conflict.kind as TreeConflict["kind"] };
  });
  return { token: data.token, revision: decodeExactU64(data.revision), name: text(data.name), files: data.files, conflicts };
}
export async function selectTreeUpload(terminal: TerminalSessionId, session: SftpSessionId, parent: string, locale: string): Promise<TreePlan | null> {
  return decodeTreePlan(await invokeTauri("select_transfer_folder_plan", { terminal, session, parent, locale }));
}
export async function selectTreeDownload(terminal: TerminalSessionId, session: SftpSessionId, entry: RemoteEntryId, locale: string): Promise<TreePlan | null> {
  return decodeTreePlan(await invokeTauri("select_transfer_folder_download_plan", { terminal, session, entry, locale }));
}
export async function resolveTree(terminal: TerminalSessionId, session: SftpSessionId, plan: TreePlan, decisions: TreeDecision[]): Promise<TreePlan | null> {
  const data = record(await invokeTauri("resolve_transfer_tree_plan", { terminal, session, token: plan.token, revision: plan.revision, decisions }), ["plan", "queued"]);
  if (typeof data.queued !== "number" || !Number.isInteger(data.queued) || data.queued < 0 || data.queued > 1024) return invalid();
  return decodeTreePlan(data.plan);
}
export async function releaseTree(plan: TreePlan): Promise<void> { await invokeTauri("release_transfer_tree_plan", { token: plan.token }); }
