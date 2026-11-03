import type { HostId, LocalTerminalOptions, TabId, TerminalLaunch, TerminalTheme } from "../terminal/contracts";

export type PaneId = string & { readonly pane: unique symbol };
export type PaneNode =
  | { kind: "pane"; paneId: PaneId; launch: TerminalLaunch }
  | { kind: "split"; direction: "horizontal" | "vertical"; ratio: number; first: PaneNode; second: PaneNode };
export type PaneLayout = { tabId: TabId; root: PaneNode };
/** "arm": first line break for this exact target set; "multiline": several lines at once. */
export type BroadcastConfirmation = "arm" | "multiline";
export type BroadcastRoute = {
  targetPaneIds: PaneId[];
  requiresConfirmation: boolean;
  confirmation: BroadcastConfirmation | null;
  blockedReason: "no_targets" | null;
};

/**
 * True when a line break is followed by more input, so one send can run more
 * than one command. A single trailing Enter is one command.
 */
export function isMultilineInput(input: string): boolean {
  return /[\r\n]/u.test(input.replace(/(?:\r\n|\r|\n)$/u, ""));
}

/**
 * Selects destinations without sending bytes or widening an empty selection.
 * Broadcast input with a line break is confirmed once per exact target set
 * (`armedPaneIds` records the confirmed set); input that spans several lines
 * is confirmed every time.
 */
export function routeInput(
  focusedPaneId: PaneId,
  selectedPaneIds: readonly PaneId[],
  broadcastEnabled: boolean,
  input: string,
  armedPaneIds: readonly PaneId[] = [],
): BroadcastRoute {
  const targetPaneIds = broadcastEnabled
    ? [...new Set(selectedPaneIds)]
    : [focusedPaneId];
  const armed = new Set(armedPaneIds);
  const confirmation: BroadcastConfirmation | null = !broadcastEnabled || targetPaneIds.length === 0 || !/[\r\n]/u.test(input) ? null
    : isMultilineInput(input) ? "multiline"
      : armed.size === targetPaneIds.length && targetPaneIds.every((id) => armed.has(id)) ? null : "arm";
  return {
    targetPaneIds,
    requiresConfirmation: confirmation !== null,
    confirmation,
    blockedReason: targetPaneIds.length === 0 ? "no_targets" : null,
  };
}

/** Converts a confirmed route into one delivery per selected pane; cancellation sends zero bytes. */
export function confirmInputRoute(route: BroadcastRoute, confirmed: boolean): PaneId[] {
  if (!confirmed || route.blockedReason !== null || route.targetPaneIds.length === 0) return [];
  return [...route.targetPaneIds];
}

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const invalid = (): never => { throw new Error("pane_contract_invalid"); };

function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalid();
  const result = value as Record<string, unknown>;
  if (Object.keys(result).length !== keys.length || !keys.every((key) => Object.hasOwn(result, key))) return invalid();
  return result;
}

function decodeLaunch(value: unknown): TerminalLaunch {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalid();
  const candidate = value as Record<string, unknown>;
  if (candidate.kind === "local") {
    const launch = record(value, ["kind", "options"]);
    if (typeof launch.options !== "object" || launch.options === null || Array.isArray(launch.options)) return invalid();
    const options = record(launch.options, Object.keys(launch.options));
    if (Object.keys(options).some((key) => key !== "shell_id" && key !== "working_directory")
      || Object.values(options).some((item) => typeof item !== "string")) return invalid();
    return { kind: "local", options: options as LocalTerminalOptions };
  }
  if (candidate.kind === "ssh") {
    const launch = record(value, ["kind", "host_id", "attempt_id", "theme"]);
    if (typeof launch.host_id !== "string" || !uuidPattern.test(launch.host_id)
      || typeof launch.attempt_id !== "string" || !uuidPattern.test(launch.attempt_id)
      || (launch.theme !== "graphite" && launch.theme !== "light")) return invalid();
    return {
      kind: "ssh",
      host_id: launch.host_id as HostId,
      attempt_id: launch.attempt_id,
      theme: launch.theme as TerminalTheme,
    };
  }
  return invalid();
}

export function decodePaneLayout(value: unknown): PaneLayout {
  const layout = record(value, ["tabId", "root"]);
  if (typeof layout.tabId !== "string" || layout.tabId.length === 0 || layout.tabId.length > 128) return invalid();
  const paneIds = new Set<string>();
  const decodeNode = (node: unknown): PaneNode => {
    if (typeof node !== "object" || node === null || Array.isArray(node)) return invalid();
    const candidate = node as Record<string, unknown>;
    if (candidate.kind === "pane") {
      const leaf = record(node, ["kind", "paneId", "launch"]);
      if (typeof leaf.paneId !== "string" || leaf.paneId.length === 0 || leaf.paneId.length > 128 || paneIds.has(leaf.paneId)) return invalid();
      paneIds.add(leaf.paneId);
      return { kind: "pane", paneId: leaf.paneId as PaneId, launch: decodeLaunch(leaf.launch) };
    }
    if (candidate.kind === "split") {
      const branch = record(node, ["kind", "direction", "ratio", "first", "second"]);
      if ((branch.direction !== "horizontal" && branch.direction !== "vertical")
        || typeof branch.ratio !== "number" || !Number.isFinite(branch.ratio) || branch.ratio < 0 || branch.ratio > 1) return invalid();
      return {
        kind: "split", direction: branch.direction, ratio: branch.ratio,
        first: decodeNode(branch.first), second: decodeNode(branch.second),
      };
    }
    return invalid();
  };
  return { tabId: layout.tabId as TabId, root: decodeNode(layout.root) };
}
