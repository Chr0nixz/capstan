import { describe, expect, it, vi } from "vitest";
import { findTerminalMatches, selectTerminalMatch } from "./search";
import type { SearchTerminal } from "./search";

function makeLine(cells: Array<{ text: string; width?: number }>) {
  return {
    length: cells.length,
    getCell(column: number) {
      const cell = cells[column];
      if (!cell) return undefined;
      return { getWidth: () => cell.width ?? 1, getChars: () => cell.text };
    },
  };
}

function makeTerminal(length: number) {
  return {
    buffer: { active: { length, get: vi.fn() } },
    clearSelection: vi.fn(),
    scrollToLine: vi.fn(),
    select: vi.fn(),
  } satisfies SearchTerminal;
}

describe("terminal search", () => {
  it("finds literal or regular-expression matches and reports invalid patterns", () => {
    const buffer = { length: 2, get: (row: number) => row === 0 ? makeLine([{ text: "a" }, { text: "." }, { text: "b" }]) : makeLine([{ text: "A" }, { text: "." }, { text: "b" }]) };
    expect(findTerminalMatches(buffer, ".", { caseSensitive: true, regex: false }).matches).toHaveLength(2);
    expect(findTerminalMatches(buffer, "a.b", { caseSensitive: false, regex: true }).matches).toHaveLength(2);
    expect(findTerminalMatches(buffer, "[", { caseSensitive: false, regex: true }).invalidExpression).toBe(true);
  });

  it("maps wide characters to terminal cells and cycles through wrapped search results", () => {
    const buffer = {
      length: 2,
      get: (row: number) => row === 0
        ? makeLine([{ text: "你", width: 2 }, { text: "", width: 0 }, { text: "x" }])
        : makeLine([{ text: "x" }, { text: "x" }]),
    };
    const foundWide = findTerminalMatches(buffer, "你", { caseSensitive: true, regex: false });
    expect(foundWide.matches).toEqual([{ row: 0, column: 0, length: 2 }]);

    const found = findTerminalMatches(buffer, "x", { caseSensitive: true, regex: false }).matches;
    const terminal = makeTerminal(2);
    const first = selectTerminalMatch(terminal, found, -1, 1, "x");
    const last = selectTerminalMatch(terminal, found, first.index, -1, "x");
    expect(first.index).toBe(0);
    expect(last.index).toBe(found.length - 1);
    expect(terminal.select).toHaveBeenLastCalledWith(1, 1, 1);
  });
});
