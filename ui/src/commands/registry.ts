import type { CommandAction } from "./contracts";

export type PaletteAction = {
  key: string;
  action: CommandAction;
  label: string;
  detail?: string;
  /** Production hosts carry the same red chip in the palette as in the host list. */
  production?: boolean;
  /** Set when the detail is pasteable data such as `user@host:port`. */
  detailMono?: boolean;
  shortcut?: string;
  keywords: readonly string[];
};

export type RegisteredAction = {
  key: string;
  action: CommandAction;
  label: string;
  searchTerms: readonly string[];
};

export type CommandCandidate = Omit<RegisteredAction, "key" | "label" | "searchTerms"> & {
  key: string;
  label: string;
  keywords: readonly string[];
};

/**
 * Every token must match a label or keyword. Labels that start with the first
 * token rank first, then labels with a word starting with it, then other label
 * matches, then keyword-only matches; ties keep registration order.
 */
export function searchCommands<T extends { label: string; keywords: readonly string[] }>(
  actions: readonly T[],
  query: string,
): T[] {
  const tokens = query.trim().toLocaleLowerCase().split(/\s+/u).filter(Boolean);
  if (tokens.length === 0) return [...actions];
  const ranked: { action: T; index: number; rank: number }[] = [];
  actions.forEach((action, index) => {
    const label = action.label.toLocaleLowerCase();
    const fields = [label, ...action.keywords.map((term) => term.toLocaleLowerCase())];
    if (!tokens.every((token) => fields.some((field) => field.includes(token)))) return;
    const first = tokens[0];
    const rank = label.startsWith(first) ? 0
      : label.split(/[\s:·@._/-]+/u).some((word) => word.startsWith(first)) ? 1
        : label.includes(first) ? 2 : 3;
    ranked.push({ action, index, rank });
  });
  return ranked.sort((left, right) => left.rank - right.rank || left.index - right.index).map(({ action }) => action);
}

const RECENT_KEY = "capstan.palette-recent";
const RECENT_LIMIT = 8;

/** Recently run action keys, newest first. Only opaque keys are stored, never labels. */
export function readRecentCommands(): string[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(value) ? value.filter((key): key is string => typeof key === "string").slice(0, RECENT_LIMIT) : [];
  } catch { return []; }
}

export function rememberCommand(key: string): void {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify([key, ...readRecentCommands().filter((item) => item !== key)].slice(0, RECENT_LIMIT)));
  } catch { /* Preference only. */ }
}
