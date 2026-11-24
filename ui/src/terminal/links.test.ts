import { describe, expect, it } from "vitest";
import { findPlainLinks, type LinkBuffer } from "./links";

/** One cell per character; CJK characters take two cells, as xterm stores them. */
function line(text: string, isWrapped = false) {
  const cells: { text: string; width: number }[] = [];
  for (const character of text) {
    if (character.charCodeAt(0) >= 0x2e80) cells.push({ text: character, width: 2 }, { text: "", width: 0 });
    else cells.push({ text: character, width: 1 });
  }
  return {
    length: cells.length,
    isWrapped,
    getCell(column: number) {
      const cell = cells[column];
      return cell ? { getWidth: () => cell.width, getChars: () => cell.text } : undefined;
    },
  };
}

function buffer(...lines: ReturnType<typeof line>[]): LinkBuffer {
  return { length: lines.length, getLine: (row) => lines[row] };
}

describe("plain-text terminal links", () => {
  it("reports a URL as a one-based, end-inclusive cell range without sentence punctuation", () => {
    expect(findPlainLinks(buffer(line("see https://example.test/a?b=1.")), 0)).toEqual([{
      text: "https://example.test/a?b=1",
      range: { start: { x: 5, y: 1 }, end: { x: 30, y: 1 } },
    }]);
  });

  it("keeps balanced brackets and drops a closing bracket opened outside the URL", () => {
    const text = (value: string): string[] => findPlainLinks(buffer(line(value)), 0).map((link) => link.text);
    expect(text("(https://example.test/x)")).toEqual(["https://example.test/x"]);
    expect(text("https://example.test/wiki/A_(b)")).toEqual(["https://example.test/wiki/A_(b)"]);
    expect(text("[docs](https://example.test/a), <https://example.test/b>"))
      .toEqual(["https://example.test/a", "https://example.test/b"]);
    expect(text("(see https://example.test/a_(b)).)")).toEqual(["https://example.test/a_(b)"]);
  });

  it("trims a long run of closing brackets in linear time", () => {
    // Recounting after every trim took over half a second for 13k brackets.
    const run = "https://example.test/(x)" + ")".repeat(40_000) + ".";
    const started = performance.now();
    expect(findPlainLinks(buffer(line(run)), 0).map((link) => link.text)).toEqual(["https://example.test/(x)"]);
    expect(performance.now() - started).toBeLessThan(500);
  });

  it("maps wide characters to cells and ends a URL at full-width punctuation", () => {
    expect(findPlainLinks(buffer(line("你好 https://example.test/a。后面")), 0)).toEqual([{
      text: "https://example.test/a",
      range: { start: { x: 6, y: 1 }, end: { x: 27, y: 1 } },
    }]);
  });

  it("follows a soft wrap from either row of the logical line", () => {
    const wrapped = buffer(line("go https://example.te"), line("st/path done", true), line("next line"));
    const expected = [{
      text: "https://example.test/path",
      range: { start: { x: 4, y: 1 }, end: { x: 7, y: 2 } },
    }];
    expect(findPlainLinks(wrapped, 0)).toEqual(expected);
    expect(findPlainLinks(wrapped, 1)).toEqual(expected);
    expect(findPlainLinks(wrapped, 2)).toEqual([]);
  });

  it("links only http(s) URLs with a host", () => {
    const text = "ssh://host mailto:a@example.test file:///etc/passwd javascript:alert(1) http:// https://";
    expect(findPlainLinks(buffer(line(text)), 0)).toEqual([]);
    expect(findPlainLinks(buffer(line("HTTPS://Example.test/Path")), 0).map((link) => link.text))
      .toEqual(["HTTPS://Example.test/Path"]);
  });

  it("skips a URL that runs past the scanned rows instead of opening a truncated address", () => {
    const rows = [line("https://example.test/"), ...Array.from({ length: 40 }, () => line("a".repeat(20), true))];
    expect(findPlainLinks(buffer(...rows), 0)).toEqual([]);
    expect(findPlainLinks(undefined, 0)).toEqual([]);
    expect(findPlainLinks(buffer(line("https://example.test/")), 3)).toEqual([]);
  });
});
