import { describe, expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M3-contract.json";
import { copy } from "../i18n";
import { decodeTunnelConfig, decodeTunnelRuntimeSnapshot, tunnelErrorCodes, tunnelStateChangedEvent } from "./contracts";
import { tunnelError } from "./errors";

describe("forwarding contract", () => {
  it("decodes local, remote and SOCKS5 configurations from the shared fixture", () => {
    expect(fixture.tunnels.map(decodeTunnelConfig).map((tunnel) => tunnel.forward.kind)).toEqual(["local", "remote", "socks5"]);
    expect(decodeTunnelRuntimeSnapshot(fixture.tunnelRuntimeSnapshot).bytes_received).toBe("18446744073709551615");
    expect(tunnelStateChangedEvent).toBe(fixture.eventNames.tunnelStateChanged);
  });

  it.each(fixture.tunnelSnapshotCases)("matches Rust runtime acceptance for $name", ({ patch, accepted }) => {
    const decode = () => decodeTunnelRuntimeSnapshot({ ...fixture.tunnelRuntimeSnapshot, ...patch });
    if (accepted) expect(decode()).toMatchObject(patch);
    else expect(decode).toThrow("forwarding_contract_invalid");
  });

  it.each(fixture.tunnelHostCases)("matches Rust endpoint acceptance for $host", ({ host, accepted }) => {
    for (const tunnel of fixture.tunnels) {
      const keys = tunnel.forward.kind === "local" ? ["bind_host", "destination_host"]
        : tunnel.forward.kind === "remote" ? ["remote_bind_host", "local_destination_host"] : ["bind_host"];
      for (const key of keys) {
        const decode = () => decodeTunnelConfig({ ...tunnel, forward: { ...tunnel.forward, [key]: host } });
        if (accepted) expect(decode().forward).toMatchObject({ [key]: host });
        else expect(decode).toThrow("forwarding_contract_invalid");
      }
    }
  });

  it("gives every runtime error code its own message in both languages", () => {
    for (const labels of [copy.en, copy["zh-CN"]]) {
      const messages = tunnelErrorCodes.map((code) => tunnelError(code, labels));
      expect(messages).not.toContain(labels.tunnelStorageFailed);
      expect(tunnelError("client_limit", labels)).toBe(labels.tunnelClientLimit);
    }
  });

  it("rejects nil configuration and host identities", () => {
    for (const key of ["id", "host_id"]) {
      expect(() => decodeTunnelConfig({ ...fixture.tunnels[0], [key]: "00000000-0000-0000-0000-000000000000" }))
        .toThrow("forwarding_contract_invalid");
    }
  });

  it("rejects invalid ports, addresses, runtime counters and undeclared fields", () => {
    for (const value of [
      { ...fixture.tunnels[0], forward: { ...fixture.tunnels[0].forward, bind_port: 0 } },
      { ...fixture.tunnels[0], forward: { ...fixture.tunnels[0].forward, destination_port: 65536 } },
      { ...fixture.tunnels[0], forward: { ...fixture.tunnels[0].forward, destination_host: "bad host" } },
      { ...fixture.tunnels[1], extra: true },
    ]) expect(() => decodeTunnelConfig(value)).toThrow("forwarding_contract_invalid");
    for (const value of [
      { ...fixture.tunnelRuntimeSnapshot, active_connections: -1 },
      { ...fixture.tunnelRuntimeSnapshot, bytes_sent: 9007199254740993 },
      { ...fixture.tunnelRuntimeSnapshot, bytes_received: "18446744073709551616" },
    ]) expect(() => decodeTunnelRuntimeSnapshot(value)).toThrow("forwarding_contract_invalid");
  });
});
