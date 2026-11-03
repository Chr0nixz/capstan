import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TerminalSessionId } from "../terminal/contracts";
import { createSftpController } from "./controller";
import { createCwdController, decodeCwdSnapshot, type CwdTransport } from "./cwd";
import { createCwdHintHub } from "./cwdHints";
import type { AttachedSftp, DirectoryPage, ExactU64, SftpSessionId } from "./contracts";
import type { SftpTransport } from "./transport";

const terminal = "terminal-1" as TerminalSessionId;
const attached: AttachedSftp = {
  sessionId: "session-1" as SftpSessionId, hostId: "host-1", connectionGeneration: "7" as ExactU64,
  path: "/home/test", atomicReplace: true, fsync: true, requestBytes: 32768,
};
const page = (path: string): DirectoryPage => ({
  sessionId: attached.sessionId, requestRevision: "1" as ExactU64, pageSequence: 0, path,
  entries: [], hasMore: false, omittedEntries: 0,
});
async function setup() {
  const transport: SftpTransport = {
    attach: vi.fn(async () => attached), detach: vi.fn(async () => undefined),
    list: vi.fn(async (_session, path) => page(path)), next: vi.fn(), release: vi.fn(),
    create: vi.fn(), prepare: vi.fn(), apply: vi.fn(),
  };
  const remote: CwdTransport = {
    update: vi.fn(async (_session, hint) => ({ sequence: hint.sequence, path: hint.uri.replace("file://host", ""), aliases: [] })),
    aliases: vi.fn(async () => ({ sequence: "0" as ExactU64, path: null, aliases: [] })),
  };
  const hub = createCwdHintHub(), publisher = hub.open(terminal);
  const browser = createSftpController(transport), cwd = createCwdController(terminal, browser, remote, hub);
  cwd.start(); await browser.start(); cwd.setActivity(true, false);
  vi.mocked(transport.list).mockClear();
  const hint = async (path: string): Promise<void> => { publisher.publish(`file://host${path}`); await vi.advanceTimersByTimeAsync(250); };
  const close = async (): Promise<void> => { cwd.dispose(); publisher.close(); await browser.dispose(); };
  return { transport, remote, browser, cwd, hint, close, publisher };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("per-terminal cwd controller", () => {
  it("keeps the latest hidden hint and navigates once when shown; manual navigation can stop follow", async () => {
    const s = await setup(); s.cwd.setActivity(false, false);
    await s.hint("/hidden-1"); await s.hint("/hidden-2");
    expect(s.remote.update).toHaveBeenCalledTimes(2);
    expect(s.transport.list).not.toHaveBeenCalled();
    s.cwd.setActivity(true, false); await Promise.resolve();
    expect(s.transport.list).toHaveBeenLastCalledWith(attached.sessionId, "/hidden-2");
    s.cwd.setFollow(false); await s.browser.navigate("/manual"); await s.hint("/latest");
    expect(s.browser.store.getState().page?.path).toBe("/manual");
    s.cwd.setFollow(true); await Promise.resolve(); await Promise.resolve();
    expect(s.transport.list).toHaveBeenLastCalledWith(attached.sessionId, "/latest");
    await s.close();
  });

  it("does not invalidate a picker or confirmation scope and resumes with the newest hint", async () => {
    const s = await setup(); s.cwd.setActivity(true, true);
    await s.hint("/one"); await s.hint("/two");
    expect(s.transport.list).not.toHaveBeenCalled();
    s.cwd.setActivity(true, false); await Promise.resolve();
    expect(s.transport.list).toHaveBeenCalledTimes(1);
    expect(s.transport.list).toHaveBeenLastCalledWith(attached.sessionId, "/two");
    await s.close();
  });

  it("serializes validation, coalesces its backlog, and ignores a response after disposal", async () => {
    const s = await setup();
    let finish!: (value: { sequence: ExactU64; path: string; aliases: string[] }) => void;
    vi.mocked(s.remote.update).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    await s.hint("/slow"); await s.hint("/obsolete"); await s.hint("/newest");
    expect(s.remote.update).toHaveBeenCalledTimes(1);
    finish({ sequence: "1" as ExactU64, path: "/slow", aliases: [] });
    await vi.advanceTimersByTimeAsync(0);
    expect(s.remote.update).toHaveBeenCalledTimes(2);
    expect(s.transport.list).toHaveBeenCalledTimes(1);
    expect(s.transport.list).toHaveBeenLastCalledWith(attached.sessionId, "/newest");
    await s.close();
    await s.hint("/after-close");
    expect(s.transport.list).toHaveBeenCalledTimes(1);
  });

  it("preserves the current list on navigation failure without repeatedly retrying the same hint", async () => {
    const s = await setup(); vi.mocked(s.transport.list).mockRejectedValueOnce({ code: "permission_denied" });
    await s.hint("/denied");
    expect(s.browser.store.getState().page?.path).toBe("/home/test");
    expect(s.browser.store.getState().error).toBe("permission_denied");
    s.cwd.setActivity(true, false);
    expect(s.transport.list).toHaveBeenCalledTimes(1);
    await s.close();
  });

  it("survives setup-cleanup-setup without duplicated subscriptions", async () => {
    const s = await setup(); s.cwd.dispose(); s.cwd.start();
    await s.hint("/strict-mode");
    expect(s.remote.update).toHaveBeenCalledTimes(1);
    expect(s.transport.list).toHaveBeenCalledTimes(1);
    await s.close();
  });

  it("ignores an old attachment's response after reattachment and sends the hint with the new binding", async () => {
    const s = await setup();
    let finish!: (value: { sequence: ExactU64; path: string; aliases: string[] }) => void;
    vi.mocked(s.remote.update).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    await s.hint("/old-operation");
    const replacement = { ...attached, sessionId: "session-2" as SftpSessionId, connectionGeneration: "8" as ExactU64 };
    s.browser.store.setState({ attached: replacement, page: { ...page("/replacement"), sessionId: replacement.sessionId } });
    finish({ sequence: "1" as ExactU64, path: "/must-not-appear", aliases: [] });
    await vi.advanceTimersByTimeAsync(0);
    expect(s.remote.update).toHaveBeenLastCalledWith(replacement, { sequence: "1", uri: "file://host/old-operation" });
    expect(vi.mocked(s.transport.list).mock.calls.every((call) => call[1] !== "/must-not-appear")).toBe(true);
    await s.close();
  });

  it("revalidates after explicit alias binding instead of accepting an earlier validation response", async () => {
    const s = await setup();
    let finish!: (value: { sequence: ExactU64; path: string; aliases: string[] }) => void;
    vi.mocked(s.remote.update).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    await s.hint("/pending");
    vi.mocked(s.remote.aliases).mockResolvedValueOnce({ sequence: "1" as ExactU64, path: "/bound", aliases: ["host"] });
    await s.cwd.aliases(["host"]);
    finish({ sequence: "1" as ExactU64, path: "/obsolete-authority", aliases: [] });
    await vi.advanceTimersByTimeAsync(0);
    expect(s.remote.update).toHaveBeenCalledTimes(2);
    expect(s.transport.list).toHaveBeenLastCalledWith(attached.sessionId, "/pending");
    expect(vi.mocked(s.transport.list).mock.calls.every((call) => call[1] !== "/obsolete-authority")).toBe(true);
    await s.close();
  });

  it("strictly decodes exact sequences and bounded metadata-only snapshots", () => {
    expect(decodeCwdSnapshot({ sequence: "18446744073709551615", path: "/中文", aliases: ["host"] }).sequence).toBe("18446744073709551615");
    for (const value of [null, { sequence: 1, path: "/", aliases: [] }, { sequence: "1", path: "relative", aliases: [] },
      { sequence: "1", path: "/", aliases: ["user@host"] }, { sequence: "1", path: "/\ud800", aliases: [] },
      { sequence: "1", path: "/\u0000", aliases: [] }, { sequence: "1", path: "/", aliases: [], fileBody: "forbidden" }]) {
      expect(() => decodeCwdSnapshot(value)).toThrow("sftp_contract_invalid");
    }
  });

  it("keeps alias validation failures identifiable while refreshing a pending hint", async () => {
    const s = await setup();
    await s.hint("/valid");
    vi.mocked(s.remote.aliases).mockRejectedValueOnce({ code: "path_invalid" });
    expect(await s.cwd.aliases(["user@host"])).toBe(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(s.cwd.store.getState().errorSource).toBe("aliases");
    expect(s.cwd.store.getState().error).toBe("path_invalid");
    await s.close();
  });
});
