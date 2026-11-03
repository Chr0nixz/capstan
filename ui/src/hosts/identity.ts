import type { GroupView, HostView } from "../terminal/contracts";

/** Names that still mark a host as production when used as a tag or a group name. */
const productionNames = new Set(["prod", "production", "生产", "生产环境"]);

function isProductionName(value: string): boolean {
  return productionNames.has(value.trim().toLocaleLowerCase());
}

export type ProductionSource = { kind: "host" } | { kind: "group"; name: string } | { kind: "tag"; name: string };
type GroupLike = Pick<GroupView, "name" | "parent_id" | "production">;

/**
 * Why a host counts as production: its own switch, a production group anywhere
 * above it, or (as a fallback for older vaults) a `prod`/`production` tag or
 * group name. Returns null for an ordinary host.
 */
export function productionSource(host: Pick<HostView, "production" | "tags" | "group_id">, groups: ReadonlyMap<string, GroupLike>): ProductionSource | null {
  if (host.production) return { kind: "host" };
  const seen = new Set<string>();
  let fallback: ProductionSource | null = null;
  for (let id = host.group_id; id && !seen.has(id); id = groups.get(id)?.parent_id ?? null) {
    seen.add(id);
    const group = groups.get(id);
    if (!group) break;
    if (group.production) return { kind: "group", name: group.name };
    if (!fallback && isProductionName(group.name)) fallback = { kind: "group", name: group.name };
  }
  const tag = host.tags.find(isProductionName);
  return fallback ?? (tag ? { kind: "tag", name: tag } : null);
}

export function isProductionHost(host: Pick<HostView, "production" | "tags" | "group_id">, groups: ReadonlyMap<string, GroupLike>): boolean {
  return productionSource(host, groups) !== null;
}

/**
 * `user@address[:port]` as every host list shows it. Port 22 is implied; any
 * other port is always shown so two entries for one address stay distinct.
 */
export function hostAddressLabel(host: Pick<HostView, "username" | "address" | "port">): string {
  return `${host.username}@${host.address}${host.port === 22 ? "" : `:${host.port}`}`;
}

const wide = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;

/**
 * Two-character legend for a host key: the initials of the first two name
 * segments ("prod-api-east" → "PA"), a numbered suffix ("build-01" → "B1"),
 * or one character when the name starts with a wide CJK glyph.
 */
export function hostMonogram(name: string): string {
  const trimmed = name.trim();
  const first = Array.from(trimmed)[0];
  if (!first) return "?";
  if (wide.test(first)) return first;
  const segments = trimmed.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  const head = Array.from(segments[0] ?? first)[0].toLocaleUpperCase();
  const next = segments[1];
  if (!next) {
    const second = Array.from(segments[0] ?? "")[1];
    return second && !wide.test(second) ? head + second.toLocaleLowerCase() : head;
  }
  if (/^\d+$/u.test(next)) return head + (next.replace(/^0+(?=\d)/u, "").slice(-1));
  const initial = Array.from(next)[0];
  return wide.test(initial) ? head : head + initial.toLocaleUpperCase();
}

export type QuickHost = { username: string; address: string; port: string };

/** Parses `user@host[:port]` (or `host:port`) typed into the host search. */
export function parseQuickHost(query: string): QuickHost | null {
  const match = /^(?:([A-Za-z0-9._-]+)@)?((?:[A-Za-z0-9-]+\.)*[A-Za-z0-9-]+|\[[0-9A-Fa-f:]+\])(?::(\d{1,5}))?$/u.exec(query.trim());
  if (!match) return null;
  const [, username = "", address, port = "22"] = match;
  if (!username && !query.includes(":") && !address.includes(".")) return null;
  const portNumber = Number(port);
  if (portNumber < 1 || portNumber > 65535) return null;
  return { username, address: address.replace(/^\[|\]$/gu, ""), port: String(portNumber) };
}
