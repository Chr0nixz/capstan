import { afterEach, describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-tree.json";
import { decodeTreePlan, releaseTree, resolveTree, selectTreeDownload, selectTreeUpload } from "./tree";
import type { TerminalSessionId } from "../terminal/contracts";
import type { RemoteEntryId, SftpSessionId } from "../sftp/contracts";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../terminal/bridge", () => ({ invokeTauri: (command: string, args: unknown) => mocks.invoke(command, args) }));
const terminal = "terminal" as TerminalSessionId, session = "session" as SftpSessionId, entry = "entry" as RemoteEntryId;
afterEach(() => mocks.invoke.mockReset());
describe("reviewed folder plans", () => {
  it("decodes the shared metadata contract without losing revision precision", () => {
    const plan = decodeTreePlan(fixture)!;
    expect(plan).toEqual(fixture);
    expect(BigInt(plan.revision)).toBe(9007199254740995n);
    expect(() => decodeTreePlan({ ...fixture, conflicts: [fixture.conflicts[0], fixture.conflicts[0]] })).toThrow();
    expect(() => decodeTreePlan({ ...fixture, localPath: "C:\\private" })).toThrow();
  });
  it("uses native picker commands and defers copying until a revision-bound decision", async () => {
    mocks.invoke.mockResolvedValue(fixture);
    await selectTreeUpload(terminal, session, "/drop", "en");
    await selectTreeDownload(terminal, session, entry, "zh-CN");
    expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual(["select_transfer_folder_plan", "select_transfer_folder_download_plan"]);
    expect(mocks.invoke).toHaveBeenCalledWith("select_transfer_folder_download_plan", { terminal, session, entry, locale: "zh-CN" });
    mocks.invoke.mockResolvedValueOnce({ plan: { ...fixture, revision: "9007199254740996" }, queued: 0 });
    const decision = [{ entry: 1, decision: { kind: "skip" as const } }];
    expect((await resolveTree(terminal, session, decodeTreePlan(fixture)!, decision))?.revision).toBe("9007199254740996");
    expect(mocks.invoke).toHaveBeenLastCalledWith("resolve_transfer_tree_plan", { terminal, session, token: fixture.token, revision: fixture.revision, decisions: decision });
    await releaseTree(decodeTreePlan(fixture)!);
    expect(mocks.invoke).toHaveBeenLastCalledWith("release_transfer_tree_plan", { token: fixture.token });
  });
});
