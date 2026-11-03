import { afterEach, describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-sftp.json";
import { decodeDirectoryPage } from "../sftp/contracts";
import { selectAndEnqueue, selectRecoveredDestination, selectSourceForRetry } from "./selection";
import type { TerminalSessionId } from "../terminal/contracts";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../terminal/bridge", () => ({ invokeTauri: (command: string, args: unknown) => mocks.invoke(command, args) }));
const page = decodeDirectoryPage(fixture), terminal = "terminal" as TerminalSessionId;
const selection = { token: "00000000-0000-4000-8000-000000000004", name: "selected.bin" };
afterEach(() => mocks.invoke.mockReset());
describe("native selections", () => {
  it("releases a picker token if the view changed while the picker was open", async () => {
    mocks.invoke.mockResolvedValueOnce(selection).mockResolvedValue(undefined);
    await selectAndEnqueue(terminal, page.sessionId, page, "en", null, () => false);
    expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual(["select_transfer_source", "release_transfer_selection"]);
  });
  it("sends only a token and remote target; a canceled picker enqueues nothing", async () => {
    mocks.invoke.mockResolvedValueOnce(selection).mockResolvedValueOnce(selection.token);
    await selectAndEnqueue(terminal, page.sessionId, page, "en", null, () => true);
    expect(mocks.invoke).toHaveBeenLastCalledWith("enqueue_transfer", { terminal, session: page.sessionId, selection: selection.token, target: { kind: "upload", parent: page.path, name: selection.name } });
    mocks.invoke.mockClear().mockResolvedValueOnce(null);
    await selectAndEnqueue(terminal, page.sessionId, page, "en", null, () => true);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });
  it("releases a selected destination when enqueue fails", async () => {
    mocks.invoke.mockResolvedValueOnce(selection).mockRejectedValueOnce({ code: "scope_expired" }).mockResolvedValueOnce(undefined);
    await expect(selectAndEnqueue(terminal, page.sessionId, page, "zh-CN", page.entries[0].id, () => true)).rejects.toEqual({ code: "scope_expired" });
    expect(mocks.invoke).toHaveBeenLastCalledWith("release_transfer_selection", { token: selection.token });
  });
  it("returns only a fresh owner-bound token for an interrupted upload retry", async () => {
    mocks.invoke.mockResolvedValueOnce(selection);
    await expect(selectSourceForRetry(terminal, page.sessionId, "/srv/drop", "en")).resolves.toBe(selection.token);
    expect(mocks.invoke).toHaveBeenCalledWith("select_transfer_source", { terminal, session: page.sessionId, parent: "/srv/drop", locale: "en" });
  });
  it("requires a new save picker grant to reattach a recovered download", async () => {
    mocks.invoke.mockResolvedValueOnce(selection);
    await expect(selectRecoveredDestination(terminal, page.sessionId, selection.token, "zh-CN")).resolves.toBe(selection.token);
    expect(mocks.invoke).toHaveBeenCalledWith("select_recovered_transfer_destination", { terminal, session: page.sessionId, id: selection.token, locale: "zh-CN" });
  });
});
