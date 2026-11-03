import { describe, expect, it } from "vitest";
import { hostAddressLabel, hostMonogram, isProductionHost, parseQuickHost, productionSource } from "./identity";
import type { GroupId } from "../terminal/contracts";

describe("host identity", () => {
  it("marks production hosts by switch, inherited group, or legacy names", () => {
    const groups = new Map([
      ["g-prod", { name: "Production", parent_id: null, production: false }],
      ["g-build", { name: "Build", parent_id: null, production: false }],
      ["g-zh", { name: "生产环境", parent_id: null, production: false }],
      ["g-live", { name: "Live", parent_id: null, production: true }],
      ["g-eu", { name: "EU", parent_id: "g-live" as GroupId, production: false }],
      ["g-loop", { name: "Loop", parent_id: "g-loop" as GroupId, production: false }],
    ]);
    const host = (production: boolean, tags: string[], group: string | null) => ({ production, tags, group_id: group as GroupId | null });
    expect(productionSource(host(true, [], "g-build"), groups)).toEqual({ kind: "host" });
    expect(productionSource(host(false, [], "g-eu"), groups)).toEqual({ kind: "group", name: "Live" });
    expect(productionSource(host(false, [], "g-prod"), groups)).toEqual({ kind: "group", name: "Production" });
    expect(productionSource(host(false, ["PROD"], null), groups)).toEqual({ kind: "tag", name: "PROD" });
    expect(isProductionHost(host(false, [], "g-zh"), groups)).toBe(true);
    expect(isProductionHost(host(false, ["production-like", "ci"], "g-build"), groups)).toBe(false);
    expect(isProductionHost(host(false, [], "missing"), groups)).toBe(false);
    expect(isProductionHost(host(false, [], "g-loop"), groups)).toBe(false);
  });

  it("builds distinct two-character host keys", () => {
    expect(hostMonogram("prod-api-east")).toBe("PA");
    expect(hostMonogram("prod-db-primary")).toBe("PD");
    expect(hostMonogram("build-01")).toBe("B1");
    expect(hostMonogram("staging web")).toBe("SW");
    expect(hostMonogram("router")).toBe("Ro");
    expect(hostMonogram("Raspberry Pi 家庭网关")).toBe("RP");
    expect(hostMonogram("家庭网关")).toBe("家");
    expect(hostMonogram("  ")).toBe("?");
  });

  it("labels addresses the same way in every host list", () => {
    expect(hostAddressLabel({ username: "deploy", address: "10.0.4.21", port: 22 })).toBe("deploy@10.0.4.21");
    expect(hostAddressLabel({ username: "postgres", address: "db1.internal", port: 2222 })).toBe("postgres@db1.internal:2222");
  });

  it("recognizes quick-add targets typed into the host search", () => {
    expect(parseQuickHost("deploy@10.0.4.21")).toEqual({ username: "deploy", address: "10.0.4.21", port: "22" });
    expect(parseQuickHost("ops@api.internal:2222")).toEqual({ username: "ops", address: "api.internal", port: "2222" });
    expect(parseQuickHost("db1.internal")).toEqual({ username: "", address: "db1.internal", port: "22" });
    expect(parseQuickHost("root@[fd00::1]:22")).toEqual({ username: "root", address: "fd00::1", port: "22" });
    expect(parseQuickHost("build")).toBeNull();
    expect(parseQuickHost("prod api")).toBeNull();
    expect(parseQuickHost("ops@host:70000")).toBeNull();
  });
});
