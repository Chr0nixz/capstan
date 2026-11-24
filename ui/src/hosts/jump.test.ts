import { describe, expect, it } from "vitest";
import type { HostId } from "../terminal/contracts";
import { jumpChain, jumpChoices } from "./jump";

const id = (value: string) => value as HostId;
const host = (name: string, jump: string | null = null) => ({ id: id(name), jump_host_id: jump ? id(jump) : null });

describe("jump hosts", () => {
  it("follows chains and stops at loops or missing hops", () => {
    const hosts = [host("gw"), host("mid", "gw"), host("db", "mid"), host("a", "b"), host("b", "a"), host("lost", "gone")];
    expect(jumpChain(hosts, hosts[2])?.map((hop) => hop.id)).toEqual(["mid", "gw"]);
    expect(jumpChain(hosts, hosts[0])).toEqual([]);
    expect(jumpChain(hosts, hosts[3])).toBeNull();
    expect(jumpChain(hosts, hosts[5])).toBeNull();
  });

  it("offers only hosts that cannot loop back or exceed the hop limit", () => {
    const hosts = [host("gw"), host("mid", "gw"), host("db", "mid"), host("h3", "db"), host("h4", "h3"), host("other")];
    // Editing gw: everything below it already goes through it.
    expect(jumpChoices(hosts, id("gw")).map((choice) => choice.id)).toEqual(["other"]);
    // A new host may use any host with room for one more hop; h4 already has four.
    expect(jumpChoices(hosts, null).map((choice) => choice.id)).toEqual(["gw", "mid", "db", "h3", "other"]);
  });
});
