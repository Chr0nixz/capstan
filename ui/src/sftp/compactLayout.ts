import type { PanelLayout } from "./panelPreferences";

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
