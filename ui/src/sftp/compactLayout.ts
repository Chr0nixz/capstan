import type { PanelLayout } from "./panelPreferences";
import type { PaneNode } from "../panes/contracts";

/** Columns most full-screen tools (htop, man, systemctl status) assume. */
export const MIN_TERMINAL_COLUMNS = 80;
/** Rows that still show a command with a screen of its output, and keep less and vim usable. */
export const MIN_TERMINAL_ROWS = 15;
/** xterm's left and right padding around the grid, from `.terminal-host`. */
const TERMINAL_PADDING = 20;
/** xterm's top and bottom padding around the grid, from `.terminal-host`. */
const TERMINAL_PADDING_Y = 16;
/** Monospace advance as a share of the font size; Cascadia Mono and Menlo are both near 0.6. */
const CELL_WIDTH_RATIO = 0.6;
/** Monospace line height as a share of the font size; both fonts measure near 1.2 in xterm. */
const CELL_HEIGHT_RATIO = 1.2;
/** A bottom-docked panel keeps this much height, but never more than half the area (`.terminal-pane-sftp-bottom`). */
const BOTTOM_PANEL_MIN = 300;

/** Share of the terminal area's width that its narrowest pane gets. */
export function narrowestPaneShare(root: PaneNode): number {
  if (root.kind === "pane") return 1;
  const first = narrowestPaneShare(root.first), second = narrowestPaneShare(root.second);
  return root.direction === "horizontal" ? Math.min(root.ratio * first, (1 - root.ratio) * second) : Math.min(first, second);
}

/**
 * Whether a right-docked panel would leave the narrowest pane under
 * MIN_TERMINAL_COLUMNS. The panel then folds, like in a narrow window.
 */
export function crowdsTerminal(workspaceWidth: number, layout: PanelLayout, root: PaneNode, fontSize: number): boolean {
  if (workspaceWidth <= 0 || layout.dock !== "right") return false;
  const terminalWidth = workspaceWidth * (1 - layout.size / 100) * narrowestPaneShare(root);
  return (terminalWidth - TERMINAL_PADDING) / (fontSize * CELL_WIDTH_RATIO) < MIN_TERMINAL_COLUMNS;
}

/** Share of the terminal area's height that its shortest pane gets. */
export function shortestPaneShare(root: PaneNode): number {
  if (root.kind === "pane") return 1;
  const first = shortestPaneShare(root.first), second = shortestPaneShare(root.second);
  return root.direction === "vertical" ? Math.min(root.ratio * first, (1 - root.ratio) * second) : Math.min(first, second);
}

/**
 * Whether a bottom-docked panel would leave the shortest pane under
 * MIN_TERMINAL_ROWS. The panel then folds, as it does for columns.
 * `areaHeight` is the height the terminal and the panel share.
 */
export function crowdsTerminalRows(areaHeight: number, layout: PanelLayout, root: PaneNode, fontSize: number): boolean {
  if (areaHeight <= 0 || layout.dock !== "bottom") return false;
  const panel = Math.max(areaHeight * layout.size / 100, Math.min(BOTTOM_PANEL_MIN, areaHeight / 2));
  const terminalHeight = (areaHeight - panel) * shortestPaneShare(root);
  return (terminalHeight - TERMINAL_PADDING_Y) / (fontSize * CELL_HEIGHT_RATIO) < MIN_TERMINAL_ROWS;
}

/**
 * Below the compact breakpoint the remote-files panel starts collapsed so the
 * terminal keeps its rows. Opening it there is a session-only choice; the
 * saved layout stays the wide-window default.
 */
export function effectiveLayout(saved: PanelLayout, compact: boolean, expanded: boolean): PanelLayout {
  return { ...saved, collapsed: saved.collapsed || (compact && !expanded) };
}

/** Splits a panel layout change into what to persist and the session-only expansion. */
export function resolveLayoutChange(saved: PanelLayout, compact: boolean, expanded: boolean, next: PanelLayout): { persist: PanelLayout | null; expanded: boolean } {
  const current = effectiveLayout(saved, compact, expanded);
  if (next.collapsed === current.collapsed) return { persist: { ...next, collapsed: saved.collapsed }, expanded };
  if (!next.collapsed) return { persist: saved.collapsed ? { ...next, collapsed: false } : null, expanded: compact || expanded };
  if (compact && !saved.collapsed) return { persist: null, expanded: false };
  return { persist: { ...next, collapsed: true }, expanded };
}
