import { describe, expect, it } from "vitest";
import { matchesHostSearch } from "./search";
import type { HostView } from "../terminal/contracts";

const host: HostView = {
  id: "host-1" as HostView["id"], name: "Build Server", address: "build.example.test", port: 22,
  username: "deploy", credential_id: "credential-1" as HostView["credential_id"], group_id: "group-1" as NonNullable<HostView["group_id"]>,
  tags: ["production", "linux"], favorite: false, revision: 0, working_directory: null,
  startup_command: null, encoding: "utf-8", theme: "graphite", agent_forwarding: false, production: false,
};

describe("host search", () => {
  it("matches ordered partial text across display name, address, tags, and group", () => {
    const groups = new Map([["group-1", "Platform" ]]);
    expect(matchesHostSearch(host, groups, "bld srv")).toBe(true);
    expect(matchesHostSearch(host, groups, "prod linux")).toBe(true);
    expect(matchesHostSearch(host, groups, "plat")).toBe(true);
    expect(matchesHostSearch(host, groups, "deploy@build")).toBe(false);
  });

  it("ignores case and diacritics and treats blank input as no filter", () => {
    expect(matchesHostSearch(host, new Map(), "  ")).toBe(true);
    expect(matchesHostSearch({ ...host, name: "Café Host" }, new Map(), "cafe host")).toBe(true);
  });
});
