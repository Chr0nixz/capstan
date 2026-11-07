import { expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M4-contract.json";
import { decodeServerStatus, matchesStatus, serverStatusEvent, statusDetailLimits, statusErrors, statusStates } from "./contracts";

it("uses the M4 status wire contract and filters by connection and generation", () => {
  const snapshot = decodeServerStatus(fixture.serverStatus);
  expect(snapshot).toEqual(fixture.serverStatus);
  expect(snapshot.memory?.total).toBe("9007199254740993");
  expect(matchesStatus(snapshot, snapshot)).toBe(true);
  expect(matchesStatus(snapshot, decodeServerStatus({ ...fixture.serverStatus, generation: "9007199254740992" }))).toBe(false);
  expect(matchesStatus(snapshot, decodeServerStatus({ ...fixture.serverStatus, connectionId: fixture.launchRequest.requestId }))).toBe(false);
  expect(statusStates).toEqual(fixture.serverStatusStates);
  expect(statusErrors).toEqual(fixture.errorCodes.serverStatus);
  expect(statusDetailLimits).toMatchObject({ interfaces: fixture.serverStatusLimits.interfaces, mounts: fixture.serverStatusLimits.mounts });
  expect(serverStatusEvent).toBe(fixture.serverStatusEvent);
});

it("rejects invalid metrics and excessive detail lists", () => {
  const sample = fixture.serverStatus;
  for (const patch of [
    { generation: "0" }, { state: "connected" }, { errorCode: "ssh_protocol" }, { cpuPercent: 101 }, { load: [0, NaN, 0] },
    { memory: { total: "7", available: "8" } }, { disk: { ...sample.disk, mount: "/var" } },
    { details: { ...sample.details, disks: Array(65).fill(sample.disk) } },
    { details: { ...sample.details, interfaces: [{ name: "eth0\u0000" }] } },
  ]) expect(() => decodeServerStatus({ ...sample, ...patch })).toThrow();
});

it("pins future M4 DTO names and settings defaults", () => {
  expect(fixture.terminalThemeSources).toEqual(["windows_terminal", "iterm2"]);
  expect(fixture.terminalTheme).toEqual({
    id: "custom:11111111-1111-4111-8111-111111111111", name: "Sample", source: "windows_terminal",
    foreground: "#f0f0f0", background: "#101010", cursor: "#ffffff", selectionBackground: "#404040",
    ansi: expect.arrayContaining(["#000000", "#ffffff"]),
  });
  expect(fixture.terminalTheme.ansi).toHaveLength(16);
  expect(fixture.launchRequestKinds).toEqual(["focus", "local", "host", "ssh"]);
  expect(fixture.launchRequest).toEqual({ requestId: "22222222-2222-4222-8222-222222222222", kind: "ssh", display: "user@example.test:22" });
  expect(fixture.settingsDefaults).toEqual({ copyOnSelect: false, systemNotifications: true, jumpListRecentHosts: false,
    quakeEnabled: false, quakeHotkey: null, quakeHeightPercent: 50, checkUpdatesOnStartup: false });
  expect(fixture.errorCodes.hello).toEqual(["hello_unavailable", "hello_canceled", "hello_key_missing", "hello_mismatch", "hello_data"]);
});
