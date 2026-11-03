import { describe, expect, it } from "vitest";
import { effectiveLayout, resolveLayoutChange } from "./compactLayout";

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
