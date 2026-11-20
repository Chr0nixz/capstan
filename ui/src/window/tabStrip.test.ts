import { describe, expect, it } from "vitest";
import { overflowEdges, revealScroll } from "./tabStrip";

describe("revealScroll", () => {
  it("keeps the offset when the tab is already in view", () => {
    expect(revealScroll(100, 300, 150, 260, 24)).toBe(100);
  });

  it("scrolls back to a tab hidden before the viewport", () => {
    expect(revealScroll(200, 300, 150, 260, 24)).toBe(126);
    expect(revealScroll(40, 300, 10, 120, 24)).toBe(0);
  });

  it("scrolls forward to a tab hidden past the viewport", () => {
    expect(revealScroll(0, 300, 400, 512, 24)).toBe(236);
  });

  it("shows the leading edge of a tab wider than the viewport", () => {
    // The 392px-window case from the audit: a 118px list hid the active tab at 112–224.
    expect(revealScroll(0, 118, 112, 224, 24)).toBe(88);
    expect(revealScroll(0, 100, 300, 488, 24)).toBe(276);
  });
});

describe("overflowEdges", () => {
  it("reports no overflow when everything fits", () => {
    expect(overflowEdges(0, 300, 300)).toEqual({ start: false, end: false });
  });

  it("reports the edges that hide tabs", () => {
    expect(overflowEdges(0, 600, 300)).toEqual({ start: false, end: true });
    expect(overflowEdges(150, 600, 300)).toEqual({ start: true, end: true });
    expect(overflowEdges(300, 600, 300)).toEqual({ start: true, end: false });
  });
});
