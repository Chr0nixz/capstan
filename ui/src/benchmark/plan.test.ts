import { describe, expect, it } from "vitest";
import { terminalPlan } from "./plan";

describe("fixed device measurement plan", () => {
  it("keeps 20 mixed tabs within the real OpenSSH per-connection session limit", () => {
    const plan = Array.from({ length: 20 }, (_, index) => terminalPlan(true, "tab_lifecycle", index));
    expect(plan.filter(item => item.backend === "local")).toHaveLength(4);
    expect(plan.filter(item => item.backend === "ssh")).toHaveLength(8);
    expect(plan.filter(item => item.backend === "ssh_alternate")).toHaveLength(8);
    expect(plan.filter(item => item.scenario === "bulk").map(item => item.backend)).toEqual(["local", "ssh"]);
  });
  it("uses a comparable isolated local case and shared-transport slow/normal SSH shells", () => {
    expect(terminalPlan(true, "bulk", 0)).toEqual(terminalPlan(false, "bulk", 0));
    expect(terminalPlan(true, "delayed_ack", 0)).toEqual({ backend: "ssh", scenario: "bulk" });
    expect(terminalPlan(true, "delayed_ack", 1)).toEqual({ backend: "ssh", scenario: "idle" });
    expect(Array.from({ length: 20 }, (_, index) => terminalPlan(false, "tab_lifecycle", index)).every(item => item.backend === "local")).toBe(true);
  });
});
