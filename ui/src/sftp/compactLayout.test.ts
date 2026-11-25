import { describe, expect, it } from "vitest";
import { crowdsTerminal, effectiveLayout, narrowestPaneShare, resolveLayoutChange } from "./compactLayout";
import type { PaneId, PaneNode } from "../panes/contracts";

const open = { dock: "right" as const, size: 36, collapsed: false };

describe("compact remote-files layout", () => {
  it("starts collapsed in a compact window without changing the saved default", () => {
    expect(effectiveLayout(open, true, false).collapsed).toBe(true);
    expect(effectiveLayout(open, false, false).collapsed).toBe(false);
    expect(effectiveLayout(open, true, true).collapsed).toBe(false);
  });

  it("keeps compact expand and collapse session-only", () => {
    expect(resolveLayoutChange(open, true, false, { ...open, collapsed: false })).toEqual({ persist: null, expanded: true });
    expect(resolveLayoutChange(open, true, true, { ...open, collapsed: true })).toEqual({ persist: null, expanded: false });
  });

  it("persists an expand when the saved layout itself was collapsed", () => {
    const saved = { ...open, collapsed: true };
    expect(resolveLayoutChange(saved, true, false, { ...saved, collapsed: false })).toEqual({ persist: { ...saved, collapsed: false }, expanded: true });
  });

  it("persists dock and size changes with the saved collapse state", () => {
    expect(resolveLayoutChange(open, true, true, { ...open, size: 48 })).toEqual({ persist: { ...open, size: 48 }, expanded: true });
    expect(resolveLayoutChange(open, false, false, { ...open, collapsed: true })).toEqual({ persist: { ...open, collapsed: true }, expanded: false });
  });
});

describe("terminal columns", () => {
  const pane = (id: string): PaneNode => ({ kind: "pane", paneId: id as PaneId, launch: { kind: "local", options: {} } });
  const right = (first: PaneNode, second: PaneNode, ratio = 0.5): PaneNode => ({ kind: "split", direction: "horizontal", ratio, first, second });
  const down = (first: PaneNode, second: PaneNode): PaneNode => ({ kind: "split", direction: "vertical", ratio: 0.5, first, second });

  it("measures the narrowest pane across side-by-side splits only", () => {
    expect(narrowestPaneShare(pane("a"))).toBe(1);
    expect(narrowestPaneShare(down(pane("a"), pane("b")))).toBe(1);
    expect(narrowestPaneShare(right(pane("a"), pane("b"), 0.7))).toBeCloseTo(0.3);
    expect(narrowestPaneShare(right(pane("a"), right(pane("b"), pane("c"))))).toBeCloseTo(0.25);
  });

  it("folds a right-docked panel that would leave a pane under 80 columns", () => {
    const layout = { dock: "right" as const, size: 30, collapsed: false };
    // 1,660 px workspace at 15 px: the terminal keeps ~127 columns.
    expect(crowdsTerminal(1660, layout, pane("a"), 15)).toBe(false);
    // The default 1,280 px window with the host list open leaves ~78.
    expect(crowdsTerminal(1020, layout, pane("a"), 15)).toBe(true);
    // A side-by-side split halves it.
    expect(crowdsTerminal(1660, layout, right(pane("a"), pane("b")), 15)).toBe(true);
    expect(crowdsTerminal(1660, layout, down(pane("a"), pane("b")), 15)).toBe(false);
    // A bottom dock never takes columns.
    expect(crowdsTerminal(1020, { ...layout, dock: "bottom" }, right(pane("a"), pane("b")), 15)).toBe(false);
    expect(crowdsTerminal(0, layout, pane("a"), 15)).toBe(false);
  });
});
