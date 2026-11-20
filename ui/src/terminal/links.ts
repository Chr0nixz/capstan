type LinkCell = { getWidth: () => number; getChars: () => string };
type LinkLine = { length: number; isWrapped?: boolean; getCell: (column: number) => LinkCell | undefined };
export type LinkBuffer = { length: number; getLine: (row: number) => LinkLine | undefined };

/** One-based, end-inclusive cell positions, as xterm's link ranges expect. */
export type TerminalLink = {
  text: string;
  range: { start: { x: number; y: number }; end: { x: number; y: number } };
};

/** Rows scanned on each side of the hovered row when a long line wraps. */
const MAX_WRAPPED_ROWS = 32;

// Full-width punctuation ends a URL: CJK prose often follows one without a space.
const URL_PATTERN = /https?:\/\/[^\s"'<>`\u0000-\u001f\u007f、。，；：！？（）【】《》「」“”‘’…]+/giu;
const TRAILING_PUNCTUATION = new Set([".", ",", ";", ":", "!", "?"]);
const CLOSING_BRACKETS: Readonly<Record<string, string>> = { ")": "(", "]": "[", "}": "{" };

function count(text: string, character: string): number {
  return text.split(character).length - 1;
}

/** Drop sentence punctuation and a closing bracket whose opener sits outside the URL. */
function trimUrl(text: string): string {
  let url = text;
  for (;;) {
    const last = url[url.length - 1];
    if (last === undefined) return url;
    const opener = CLOSING_BRACKETS[last];
    if (TRAILING_PUNCTUATION.has(last) || (opener !== undefined && count(url, last) > count(url, opener))) {
      url = url.slice(0, -1);
    } else return url;
  }
}

function isWebUrl(text: string): boolean {
  try {
    const url = new URL(text);
    return (url.protocol === "http:" || url.protocol === "https:") && url.hostname.length > 0;
  } catch {
    return false;
  }
}

/**
 * Finds plain-text http(s) URLs on the logical line containing `row`
 * (zero-based), following soft wraps. Other schemes stay unlinked unless the
 * remote marks them explicitly with OSC 8.
 */
export function findPlainLinks(buffer: LinkBuffer | undefined, row: number): TerminalLink[] {
  if (!buffer || !buffer.getLine(row)) return [];
  let first = row;
  while (first > 0 && row - first < MAX_WRAPPED_ROWS && buffer.getLine(first)?.isWrapped) first -= 1;
  let last = row;
  while (last + 1 < buffer.length && last - row < MAX_WRAPPED_ROWS && buffer.getLine(last + 1)?.isWrapped) last += 1;
  const truncated = !!buffer.getLine(last + 1)?.isWrapped;

  let text = "";
  const cells: { row: number; start: number; end: number }[] = [];
  for (let current = first; current <= last; current += 1) {
    const line = buffer.getLine(current);
    if (!line) continue;
    for (let column = 0; column < line.length; ) {
      const cell = line.getCell(column);
      const width = cell ? Math.max(0, cell.getWidth()) : 0;
      if (!cell || width === 0) { column += 1; continue; }
      const chars = cell.getChars() || " ";
      for (let index = 0; index < chars.length; index += 1) {
        text += chars[index];
        cells.push({ row: current, start: column, end: column + width });
      }
      column += width;
    }
  }

  const links: TerminalLink[] = [];
  for (const match of text.matchAll(URL_PATTERN)) {
    // A URL running past the scanned rows would open as a truncated address.
    if (truncated && match.index + match[0].length === text.length) continue;
    const url = trimUrl(match[0]);
    const start = cells[match.index];
    const end = cells[match.index + url.length - 1];
    if (!start || !end || !isWebUrl(url)) continue;
    links.push({
      text: url,
      range: { start: { x: start.start + 1, y: start.row + 1 }, end: { x: end.end, y: end.row + 1 } },
    });
  }
  return links;
}
