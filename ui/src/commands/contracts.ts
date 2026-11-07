import type { ConnectionId } from "../connections/contracts";
import type { SnippetId } from "../snippets/contracts";
import type { HostId, TerminalTheme } from "../terminal/contracts";
import { isTerminalTheme } from "../themes/contracts";

type ActionMetadata = { labelKey: string; keywordKeys: string[] };
/** Workspace operations the palette can trigger; each maps to an existing shortcut or toolbar key. */
export const workspaceCommands = [
  "new_terminal", "close_terminal", "split_right", "split_down", "toggle_broadcast", "toggle_sidebar", "focus_files", "focus_transfers", "lock_vault",
] as const;
export type WorkspaceCommand = (typeof workspaceCommands)[number];
export type CommandAction =
  | (ActionMetadata & { id: "connect_host"; hostId: HostId })
  | (ActionMetadata & { id: "run_snippet"; snippetId: SnippetId; connectionId?: ConnectionId })
  | (ActionMetadata & { id: "set_theme"; theme: TerminalTheme })
  | (ActionMetadata & { id: "toggle_panel"; panel: "sessions" | "sftp" | "forwarding" | "snippets" })
  | (ActionMetadata & { id: "open_settings" })
  | (ActionMetadata & { id: "open_shell"; shellId: string })
  | (ActionMetadata & { id: "workspace"; command: WorkspaceCommand });

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const keyPattern = /^[A-Za-z][A-Za-z0-9]*(?:\.[A-Za-z][A-Za-z0-9]*)*$/u;
const invalid = (): never => { throw new Error("command_action_invalid"); };

function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalid();
  const result = value as Record<string, unknown>;
  if (Object.keys(result).length !== keys.length || !keys.every((key) => Object.hasOwn(result, key))) return invalid();
  return result;
}

function metadata(action: Record<string, unknown>): ActionMetadata {
  if (typeof action.labelKey !== "string" || !keyPattern.test(action.labelKey)
    || !Array.isArray(action.keywordKeys) || action.keywordKeys.length > 16
    || action.keywordKeys.some((key) => typeof key !== "string" || !keyPattern.test(key))) return invalid();
  return { labelKey: action.labelKey, keywordKeys: action.keywordKeys };
}

export function decodeCommandAction(value: unknown): CommandAction {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalid();
  const candidate = value as Record<string, unknown>;
  if (candidate.id === "connect_host") {
    const action = record(value, ["id", "labelKey", "keywordKeys", "hostId"]);
    if (typeof action.hostId !== "string" || !uuidPattern.test(action.hostId)) return invalid();
    return { ...metadata(action), id: "connect_host", hostId: action.hostId as HostId };
  }
  if (candidate.id === "run_snippet") {
    const keys = Object.hasOwn(candidate, "connectionId")
      ? ["id", "labelKey", "keywordKeys", "snippetId", "connectionId"]
      : ["id", "labelKey", "keywordKeys", "snippetId"];
    const action = record(value, keys);
    if (typeof action.snippetId !== "string" || !uuidPattern.test(action.snippetId)
      || (keys.includes("connectionId") && (typeof action.connectionId !== "string" || !uuidPattern.test(action.connectionId)))) return invalid();
    return {
      ...metadata(action), id: "run_snippet", snippetId: action.snippetId as SnippetId,
      ...(typeof action.connectionId === "string" ? { connectionId: action.connectionId as ConnectionId } : {}),
    };
  }
  if (candidate.id === "set_theme") {
    const action = record(value, ["id", "labelKey", "keywordKeys", "theme"]);
    if (!isTerminalTheme(action.theme)) return invalid();
    return { ...metadata(action), id: "set_theme", theme: action.theme };
  }
  if (candidate.id === "toggle_panel") {
    const action = record(value, ["id", "labelKey", "keywordKeys", "panel"]);
    if (typeof action.panel !== "string" || !["sessions", "sftp", "forwarding", "snippets"].includes(action.panel)) return invalid();
    return { ...metadata(action), id: "toggle_panel", panel: action.panel as "sessions" | "sftp" | "forwarding" | "snippets" };
  }
  if (candidate.id === "open_settings") {
    const action = record(value, ["id", "labelKey", "keywordKeys"]);
    return { ...metadata(action), id: "open_settings" };
  }
  if (candidate.id === "open_shell") {
    const action = record(value, ["id", "labelKey", "keywordKeys", "shellId"]);
    // Shell IDs come from local discovery (for example "pwsh" or "wsl:Ubuntu").
    if (typeof action.shellId !== "string" || action.shellId.length === 0 || action.shellId.length > 256 || /\p{Cc}/u.test(action.shellId)) return invalid();
    return { ...metadata(action), id: "open_shell", shellId: action.shellId };
  }
  if (candidate.id === "workspace") {
    const action = record(value, ["id", "labelKey", "keywordKeys", "command"]);
    if (typeof action.command !== "string" || !(workspaceCommands as readonly string[]).includes(action.command)) return invalid();
    return { ...metadata(action), id: "workspace", command: action.command as WorkspaceCommand };
  }
  return invalid();
}
