import type { Run, Scenario } from "./contracts";

export type FixtureBackend = "local" | "ssh" | "ssh_alternate";

/** Keep each SSH identity below OpenSSH's default MaxSessions=10. */
export function terminalPlan(mixed: boolean, kind: Run["kind"], index: number): { backend: FixtureBackend; scenario: Scenario } {
  const backend: FixtureBackend = !mixed || kind === "bulk" ? "local"
    : kind === "delayed_ack" ? "ssh"
    : index % 5 === 0 ? "local" : index % 2 === 0 ? "ssh_alternate" : "ssh";
  return { backend, scenario: index === 0 || (mixed && kind === "tab_lifecycle" && index === 1) ? "bulk" : "idle" };
}
