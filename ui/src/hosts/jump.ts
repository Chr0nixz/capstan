import type { HostId, HostView } from "../terminal/contracts";

/** Hops a connection may pass through; matches the Vault's limit. */
export const maxJumpHops = 4;

type Hop = Pick<HostView, "id" | "jump_host_id">;

/** The hosts `host` is reached through, nearest first, or null if a hop is missing or the chain loops. */
export function jumpChain<T extends Hop>(hosts: readonly T[], host: Hop): T[] | null {
  const chain: T[] = [];
  let next = host.jump_host_id;
  while (next) {
    const id: HostId = next;
    if (id === host.id || chain.some((hop) => hop.id === id) || chain.length === maxJumpHops) return null;
    const hop = hosts.find((candidate) => candidate.id === id);
    if (!hop) return null;
    chain.push(hop);
    next = hop.jump_host_id;
  }
  return chain;
}

/**
 * Hosts that `editing` (or a new host, when null) can use as its jump host:
 * not itself, not a host that already goes through it, and not one whose own
 * chain would leave no room for this hop.
 */
export function jumpChoices<T extends Hop>(hosts: readonly T[], editing: HostId | null): T[] {
  return hosts.filter((host) => {
    if (host.id === editing) return false;
    const chain = jumpChain(hosts, host);
    return chain !== null && chain.length < maxJumpHops && !chain.some((hop) => hop.id === editing);
  });
}
