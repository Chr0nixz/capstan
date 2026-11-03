import { invokeTauri } from "../terminal/bridge";
import type { TerminalSessionId } from "../terminal/contracts";
import type { RemoteEntryId, SftpSessionId } from "../sftp/contracts";
import { decodeTreePlan, type TreePlan } from "./tree";
export type NativeExport = { token: string; name: string; state: "planning" | "preparing" | "ready" | "dragging" | "retained" | "failed" };
function invalid(): never { throw { code: "remote_io" }; }
let dropTargetQueue = Promise.resolve();
export function armDrop(terminal: TerminalSessionId, session: SftpSessionId, parent: string): Promise<string> {
  const operation = dropTargetQueue.then(async () => {
    const token = await invokeTauri("arm_native_drop", { terminal, session, parent });
    if (typeof token !== "string" || !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu.test(token)) return invalid();
    return token;
  });
  dropTargetQueue = operation.then(() => undefined, () => undefined);
  return operation;
}
export function disarmDrop(token: string): Promise<void> {
  const operation = dropTargetQueue.then(() => invokeTauri("disarm_native_drop", { token }));
  dropTargetQueue = operation.catch(() => undefined);
  return operation;
}
export function decodeExport(value: unknown): NativeExport {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  const data = value as Record<string, unknown>;
  if (Object.keys(data).length !== 3 || typeof data.token !== "string" || !/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu.test(data.token)
    || typeof data.name !== "string" || !data.name || data.name.length > 4096 || /[\u0000-\u001f\u007f]/u.test(data.name)
    || !["planning", "preparing", "ready", "dragging", "retained", "failed"].includes(String(data.state))) return invalid();
  return { token: data.token, name: data.name, state: data.state as NativeExport["state"] };
}
export async function nativeExports(): Promise<NativeExport[]> {
  const value: unknown = await invokeTauri("native_export_snapshot");
  if (!Array.isArray(value) || value.length > 8) return invalid();
  return value.map(decodeExport);
}
export async function prepareExport(terminal: TerminalSessionId, session: SftpSessionId, entry: RemoteEntryId): Promise<{ export: NativeExport; plan: TreePlan }> {
  const result: unknown = await invokeTauri("prepare_native_export", { terminal, session, entry });
  if (!result || typeof result !== "object") return invalid();
  const data = result as Record<string, unknown>;
  if (Object.keys(data).length !== 2 || !Object.hasOwn(data, "export") || !Object.hasOwn(data, "plan")) return invalid();
  return { export: decodeExport(data.export), plan: decodeTreePlan(data.plan) ?? invalid() };
}
export async function startExport(terminal: TerminalSessionId, session: SftpSessionId, token: string): Promise<void> {
  await invokeTauri("start_native_export", { terminal, session, token });
}
export async function releaseExport(token: string): Promise<void> { await invokeTauri("release_native_export", { token }); }
export type NativeDrop = { token: string; x: number; y: number };
export function insideNativeDrop(value: unknown, rect: Pick<DOMRect, "left" | "right" | "top" | "bottom">): value is NativeDrop {
  if (!value || typeof value !== "object") return false;
  const data = value as Record<string, unknown>;
  return Object.keys(data).length === 3 && typeof data.token === "string" && /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu.test(data.token) && typeof data.x === "number" && Number.isFinite(data.x)
    && typeof data.y === "number" && Number.isFinite(data.y) && data.x >= rect.left && data.x < rect.right && data.y >= rect.top && data.y < rect.bottom;
}
export async function enqueueDrop(terminal: TerminalSessionId, session: SftpSessionId, parent: string, token: string): Promise<TreePlan[]> {
  const result = await invokeTauri("enqueue_native_drop", { terminal, session, parent, token });
  if (!result || typeof result !== "object") return invalid();
  const data = result as Record<string, unknown>;
  if (Object.keys(data).length !== 2 || !Array.isArray(data.plans) || data.plans.length > 8 || typeof data.queued !== "number" || !Number.isInteger(data.queued) || data.queued < 0 || data.queued > 256) return invalid();
  return data.plans.map((value) => decodeTreePlan(value) ?? invalid());
}
