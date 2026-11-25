import { describe, expect, it } from "vitest";
import { crowdsTerminal, crowdsTerminalRows, effectiveLayout, narrowestPaneShare, resolveLayoutChange, shortestPaneShare } from "./compactLayout";
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

describe("terminal rows", () => {
  const pane = (id: string): PaneNode => ({ kind: "pane", paneId: id as PaneId, launch: { kind: "local", options: {} } });
  const right = (first: PaneNode, second: PaneNode): PaneNode => ({ kind: "split", direction: "horizontal", ratio: 0.5, first, second });
  const down = (first: PaneNode, second: PaneNode, ratio = 0.5): PaneNode => ({ kind: "split", direction: "vertical", ratio, first, second });
  const bottom = { dock: "bottom" as const, size: 42, collapsed: false };

  it("measures the shortest pane across stacked splits only", () => {
    expect(shortestPaneShare(pane("a"))).toBe(1);
    expect(shortestPaneShare(right(pane("a"), pane("b")))).toBe(1);
    expect(shortestPaneShare(down(pane("a"), pane("b"), 0.7))).toBeCloseTo(0.3);
    expect(shortestPaneShare(down(pane("a"), down(pane("b"), pane("c"))))).toBeCloseTo(0.25);
  });

  it("folds a bottom-docked panel that would leave a pane under 15 rows", () => {
    // The default 1,280 x 800 window leaves 665 px: the panel takes 300 and the terminal keeps ~19 rows.
    expect(crowdsTerminalRows(665, bottom, pane("a"), 15)).toBe(false);
    // The panel never takes more than half, so 600 px still leaves 15 rows; 560 px does not.
    expect(crowdsTerminalRows(600, bottom, pane("a"), 15)).toBe(false);
    expect(crowdsTerminalRows(560, bottom, pane("a"), 15)).toBe(true);
    // A short window, or the transfer queue opened under a 720 x 560 window (~275 px).
    expect(crowdsTerminalRows(365, bottom, pane("a"), 15)).toBe(true);
    expect(crowdsTerminalRows(275, bottom, pane("a"), 15)).toBe(true);
    // A stacked split halves the rows; a side-by-side split does not.
    expect(crowdsTerminalRows(665, bottom, down(pane("a"), pane("b")), 15)).toBe(true);
    expect(crowdsTerminalRows(665, bottom, right(pane("a"), pane("b")), 15)).toBe(false);
    // A larger font needs more height for the same rows.
    expect(crowdsTerminalRows(665, bottom, pane("a"), 20)).toBe(true);
  });

  it("leaves a right dock and an unmeasured area alone", () => {
    expect(crowdsTerminalRows(275, { ...bottom, dock: "right" }, pane("a"), 15)).toBe(false);
    expect(crowdsTerminalRows(0, bottom, pane("a"), 15)).toBe(false);
  });
});
