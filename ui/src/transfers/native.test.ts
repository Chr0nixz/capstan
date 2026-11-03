import { afterEach, describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-tree.json";
import { armDrop, decodeExport, disarmDrop, enqueueDrop, insideNativeDrop, nativeExports, prepareExport, releaseExport, startExport } from "./native";
import type { TerminalSessionId } from "../terminal/contracts";
import type { RemoteEntryId, SftpSessionId } from "../sftp/contracts";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../terminal/bridge", () => ({ invokeTauri: (command: string, args: unknown) => mocks.invoke(command, args) }));
const terminal = "terminal" as TerminalSessionId, session = "session" as SftpSessionId, entry = "entry" as RemoteEntryId;
const exported = { token: fixture.token, name: "folder", state: "planning" };
afterEach(() => mocks.invoke.mockReset());
describe("native drop/export metadata", () => {
  it("serializes visible-target arming and releases only the returned target lease", async () => {
    let release!: (value: string) => void;
    mocks.invoke.mockReturnValueOnce(new Promise<string>((resolve) => { release = resolve; })).mockResolvedValue(undefined);
    const arm = armDrop(terminal, session, "/drop");
    const disarm = disarmDrop(fixture.token);
    await Promise.resolve();
    expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual(["arm_native_drop"]);
    release(fixture.token);
    await expect(arm).resolves.toBe(fixture.token); await disarm;
    expect(mocks.invoke).toHaveBeenLastCalledWith("disarm_native_drop", { token: fixture.token });
  });
  it("accepts only native receipts inside the visible panel and never accepts path grants", () => {
    const rect = { left: 10, right: 100, top: 20, bottom: 200 };
    expect(insideNativeDrop({ token: fixture.token, x: 10, y: 20 }, rect)).toBe(true);
    for (const value of [{ token: fixture.token, x: 100, y: 21 }, { token: fixture.token, x: NaN, y: 21 }, { token: fixture.token, x: 11, y: 21, paths: ["C:\\private"] }, { token: "-".repeat(36), x: 11, y: 21 }]) expect(insideNativeDrop(value, rect)).toBe(false);
  });
  it("keeps folder decisions on the drop path and exposes only staged export tokens", async () => {
    mocks.invoke.mockResolvedValueOnce({ queued: 1, plans: [fixture] });
    expect(await enqueueDrop(terminal, session, "/drop", fixture.token)).toEqual([fixture]);
    expect(mocks.invoke).toHaveBeenLastCalledWith("enqueue_native_drop", { terminal, session, parent: "/drop", token: fixture.token });
    mocks.invoke.mockResolvedValueOnce({ export: exported, plan: fixture });
    expect(await prepareExport(terminal, session, entry)).toEqual({ export: exported, plan: fixture });
    mocks.invoke.mockResolvedValueOnce([exported]);
    expect(await nativeExports()).toEqual([exported]);
    mocks.invoke.mockResolvedValueOnce(false);
    await startExport(terminal, session, fixture.token);
    expect(mocks.invoke.mock.calls.some(([command]) => command === "release_native_export")).toBe(false);
    await releaseExport(fixture.token);
    expect(mocks.invoke).toHaveBeenLastCalledWith("release_native_export", { token: fixture.token });
    expect(() => decodeExport({ ...exported, path: "C:\\private" })).toThrow();
  });
});
