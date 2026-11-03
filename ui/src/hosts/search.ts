import type { HostView } from "../terminal/contracts";

function normalize(value: string): string {
  return value.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLocaleLowerCase();
}

function containsInOrder(candidate: string, query: string): boolean {
  if (candidate.includes(query)) return true;
  let queryIndex = 0;
  for (const character of candidate) {
    if (character === query[queryIndex]) queryIndex += 1;
    if (queryIndex === query.length) return true;
  }
  return false;
}

export function matchesHostSearch(host: HostView, groupNames: ReadonlyMap<string, string>, query: string): boolean {
  const tokens = normalize(query).trim().split(/\s+/u).filter(Boolean);
  if (tokens.length === 0) return true;
  const fields = [host.name, host.address, host.username, ...host.tags, groupNames.get(host.group_id ?? "") ?? ""]
    .map(normalize);
  return tokens.every((token) => fields.some((field) => containsInOrder(field, token)));
}
