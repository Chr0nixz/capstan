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
    // Editing gw: everything below it already goes through it, and h4 is
    // already four hops away, so gw cannot take a jump host of its own.
    expect(jumpChoices(hosts, id("gw")).map((choice) => choice.id)).toEqual([]);
    // h4 is three hops below mid, so mid may only use a host without a jump host.
    expect(jumpChoices(hosts, id("mid")).map((choice) => choice.id)).toEqual(["gw", "other"]);
    // A new host may use any host with room for one more hop; h4 already has four.
    expect(jumpChoices(hosts, null).map((choice) => choice.id)).toEqual(["gw", "mid", "db", "h3", "other"]);
  });

  it("leaves room for the hosts already reached through the edited host", () => {
    // gw <- mid <- db is a chain of two hops; edge has two hosts below it.
    const hosts = [host("gw"), host("mid", "gw"), host("db", "mid"), host("edge"), host("app", "edge"), host("api", "app")];
    // Through db, api would need db, mid, gw, edge and app: five hops.
    expect(jumpChoices(hosts, id("edge")).map((choice) => choice.id)).toEqual(["gw", "mid"]);
    // app has one host below it, so db (two hops of its own) still fits.
    expect(jumpChoices(hosts, id("app")).map((choice) => choice.id)).toEqual(["gw", "mid", "db", "edge"]);
    // A host with nothing below it may use any host with room for one more hop.
    expect(jumpChoices(hosts, id("api")).map((choice) => choice.id)).toEqual(["gw", "mid", "db", "edge", "app"]);
  });
});
