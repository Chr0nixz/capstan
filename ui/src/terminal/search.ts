export type TerminalSearchOptions = { caseSensitive: boolean; regex: boolean };
export type TerminalMatch = { row: number; column: number; length: number };
export type TerminalSearchResult = {
  index: number;
  count: number;
  match: TerminalMatch | null;
  invalidExpression: boolean;
};

type SearchCell = { getWidth: () => number; getChars: () => string };
type SearchLine = { length: number; getCell: (column: number) => SearchCell | undefined };
type SearchBuffer = { length: number; get: (row: number) => SearchLine | undefined };
export type SearchTerminal = {
  buffer?: { active?: SearchBuffer };
  clearSelection: () => void;
  scrollToLine: (line: number) => void;
  select: (column: number, row: number, length: number) => void;
};

type CellText = { text: string; starts: number[]; ends: number[] };

function lineText(line: SearchLine): CellText {
  let text = "";
  const starts: number[] = [];
  const ends: number[] = [];
  for (let column = 0; column < line.length; ) {
    const cell = line.getCell(column);
    if (!cell) { column += 1; continue; }
    const width = Math.max(0, cell.getWidth());
    if (width === 0) { column += 1; continue; }
    const chars = cell.getChars() || " ";
    for (let index = 0; index < chars.length; index += 1) {
      text += chars[index];
      starts.push(column);
      ends.push(column + width);
    }
    column += width;
  }
  return { text, starts, ends };
}

function createExpression(query: string, options: TerminalSearchOptions): RegExp {
  const source = options.regex
    ? query
    : query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(source, options.caseSensitive ? "gu" : "giu");
}

export function findTerminalMatches(
  buffer: SearchBuffer | undefined,
  query: string,
  options: TerminalSearchOptions,
): { matches: TerminalMatch[]; invalidExpression: boolean } {
  if (!buffer || query.length === 0) return { matches: [], invalidExpression: false };
  let expression: RegExp;
  try {
    expression = createExpression(query, options);
  } catch {
    return { matches: [], invalidExpression: true };
  }
  const matches: TerminalMatch[] = [];
  for (let row = 0; row < buffer.length; row += 1) {
    const line = buffer.get(row);
    if (!line) continue;
    const content = lineText(line);
    expression.lastIndex = 0;
    let result = expression.exec(content.text);
    while (result) {
      if (result[0].length > 0) {
        const column = content.starts[result.index];
        const end = content.ends[result.index + result[0].length - 1];
        if (column !== undefined && end !== undefined && end > column) {
          matches.push({ row, column, length: end - column });
        }
      }
      if (!expression.global) break;
      if (result[0].length === 0) expression.lastIndex += 1;
      result = expression.exec(content.text);
    }
  }
  return { matches, invalidExpression: false };
}

export function selectTerminalMatch(
  terminal: SearchTerminal,
  matches: TerminalMatch[],
  currentIndex: number,
  direction: 1 | -1,
  query: string,
): TerminalSearchResult {
  if (!query) {
    terminal.clearSelection();
    return { index: -1, count: 0, match: null, invalidExpression: false };
  }
  if (matches.length === 0) {
    terminal.clearSelection();
    return { index: -1, count: 0, match: null, invalidExpression: false };
  }
  const initial = direction > 0 ? 0 : matches.length - 1;
  const index = currentIndex < 0
    ? initial
    : (currentIndex + direction + matches.length) % matches.length;
  const match = matches[index];
  terminal.clearSelection();
  terminal.scrollToLine(match.row);
  terminal.select(match.column, match.row, match.length);
  return { index, count: matches.length, match, invalidExpression: false };
}
