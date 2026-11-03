import { describe, expect, it, vi } from "vitest";
import { createSftpController } from "./controller";
import type { AttachedSftp, DirectoryPage, ExactU64, RemoteEntryId } from "./contracts";
import type { SftpTransport } from "./transport";

const attached = { sessionId: "00000000-0000-4000-8000-000000000001", hostId: "00000000-0000-4000-8000-000000000002", connectionGeneration: "1", path: "/", atomicReplace: true, fsync: true, requestBytes: 32768 } as AttachedSftp;
const page = (sequence: number, name: string): DirectoryPage => ({ sessionId: attached.sessionId, requestRevision: "1" as ExactU64, pageSequence: sequence, path: "/", entries: [{ id: `00000000-0000-4000-8000-${(sequence + 3).toString().padStart(12, "0")}` as RemoteEntryId, name, metadata: { kind: "file", size: "1" as ExactU64, modifiedSeconds: null, permissions: 0o600 } }], hasMore: sequence === 0, omittedEntries: 0 });
function transport(overrides: Partial<SftpTransport> = {}): SftpTransport {
  return { attach: vi.fn(async () => attached), detach: vi.fn(async () => undefined), list: vi.fn(async () => page(0, "first")), next: vi.fn(async () => page(1, "second")), release: vi.fn(async () => undefined), create: vi.fn(async () => undefined), prepare: vi.fn(async () => ({ token: "token", hostId: attached.hostId, path: "/first", metadata: page(0, "first").entries[0].metadata, mutation: { kind: "delete" as const } })), apply: vi.fn(async () => undefined), ...overrides };
}

describe("SFTP controller", () => {
  it("drops a late navigation result after a newer request", async () => {
    let resolveOld: ((value: DirectoryPage) => void) | undefined;
    const old = new Promise<DirectoryPage>((resolve) => { resolveOld = resolve; });
    const api = transport({ list: vi.fn((_session, path: string) => path === "/old" ? old : Promise.resolve(page(0, "new"))) });
    const controller = createSftpController(api);
    await controller.start();
    const first = controller.navigate("/old");
    await Promise.resolve();
    const second = controller.navigate("/new");
    resolveOld?.(page(0, "old"));
    await Promise.all([first, second]);
    expect(controller.store.getState().page?.entries[0].name).toBe("new");
  });
  it("keeps recent pages and their entry capabilities for backward navigation", async () => {
    const api = transport();
    const controller = createSftpController(api);
    await controller.start();
    await controller.next();
    expect(api.release).not.toHaveBeenCalled();
    expect(controller.store.getState().page?.pageSequence).toBe(1);
    await controller.previous();
    expect(controller.store.getState().page?.entries[0].name).toBe("first");
    await controller.next();
    expect(api.next).toHaveBeenCalledTimes(1);
  });
  it("recovers the last server scope when a newer navigation fails after an older one commits", async () => {
    let resolveOld: (value: DirectoryPage) => void = () => undefined;
    const older = new Promise<DirectoryPage>((resolve) => { resolveOld = resolve; });
    const api = transport({ list: vi.fn((_session, path) => path === "/old" ? older : path === "/missing" ? Promise.reject({ code: "not_found" }) : Promise.resolve(page(0, "initial"))) });
    const controller = createSftpController(api);
    await controller.start();
    const first = controller.navigate("/old");
    await Promise.resolve();
    const last = controller.navigate("/missing");
    resolveOld({ ...page(0, "old"), path: "/old", requestRevision: "2" as ExactU64 });
    await Promise.all([first, last]);
    expect(controller.store.getState()).toMatchObject({ busy: false, error: "not_found", page: { path: "/old", requestRevision: "2" } });
  });
  it("detaches an attached session when disposed", async () => {
    const api = transport();
    const controller = createSftpController(api);
    await controller.start();
    await controller.dispose();
    expect(api.detach).toHaveBeenCalledWith(attached.sessionId);
    expect(controller.store.getState().attached).toBeNull();
  });
  it("can reattach after effect cleanup and restart", async () => {
    const api = transport();
    const controller = createSftpController(api);
    await controller.start();
    await controller.dispose();
    await controller.start();
    expect(api.attach).toHaveBeenCalledTimes(2);
    expect(controller.store.getState().attached?.sessionId).toBe(attached.sessionId);
    await controller.dispose();
  });
  it("maps attach failures to stable codes and allows an explicit retry", async () => {
    const api = transport({ attach: vi.fn().mockRejectedValueOnce({ code: "sftp_unavailable" }).mockResolvedValue(attached) });
    const controller = createSftpController(api);
    await controller.start();
    expect(controller.store.getState().error).toBe("sftp_unavailable");
    expect(controller.store.getState().attached).toBeNull();
    await controller.start();
    expect(controller.store.getState().error).toBeNull();
    expect(controller.store.getState().page?.path).toBe("/");
    await controller.dispose();
  });
  it("keeps a failed creation visible as a stable remote error", async () => {
    const api = transport({ create: vi.fn().mockRejectedValue({ code: "permission_denied", message: "/secret/path" }) });
    const controller = createSftpController(api);
    await controller.start();
    expect(await controller.create("new.txt", false)).toBe(false);
    expect(controller.store.getState().error).toBe("permission_denied");
    expect(controller.store.getState().page?.path).toBe("/");
    await controller.dispose();
  });

  it("bounds page history and re-enumerates an evicted previous page", async () => {
    let sequence = 0;
    const api = transport({
      list: vi.fn(async () => { sequence = 0; return { ...page(0, "file-0"), hasMore: true }; }),
      next: vi.fn(async () => { ++sequence; return { ...page(sequence, `file-${sequence}`), hasMore: sequence < 11 }; }),
    });
    const controller = createSftpController(api);
    await controller.start();
    for (let index = 0; index < 10; index++) await controller.next();
    expect(api.release).toHaveBeenCalledTimes(3);
    for (let index = 0; index < 8; index++) await controller.previous();
    expect(api.list).toHaveBeenCalledTimes(2);
    expect(controller.store.getState().page?.pageSequence).toBe(2);
    expect(controller.store.getState().page?.entries[0].name).toBe("file-2");
  });

  it("preserves a usable page after failed navigation and maps a missing path", async () => {
    const api = transport({ list: vi.fn().mockResolvedValueOnce(page(0, "first")).mockRejectedValue({ code: "not_found" }) });
    const controller = createSftpController(api);
    await controller.start();
    await controller.navigate("/missing");
    expect(controller.store.getState().error).toBe("not_found");
    controller.select(page(0, "first").entries[0].id);
    expect(await controller.prepare({ kind: "delete" })).not.toBeNull();
  });

  it("does not suggest repeating a successful create when refreshing fails", async () => {
    const api = transport({ list: vi.fn().mockResolvedValueOnce(page(0, "first")).mockRejectedValue({ code: "permission_denied" }) });
    const controller = createSftpController(api);
    await controller.start();
    expect(await controller.create("new", false)).toBe(true);
    expect(controller.store.getState().error).toBe("permission_denied");
    expect(api.create).toHaveBeenCalledTimes(1);
  });

  it("rejects an out-of-sequence page without leaving the view busy", async () => {
    const api = transport({ next: vi.fn(async () => page(3, "invalid")) });
    const controller = createSftpController(api);
    await controller.start();
    await controller.next();
    expect(controller.store.getState()).toMatchObject({ busy: false, error: "scope_expired", page: null });
  });

  it("ignores an operation failure after disposal", async () => {
    let reject: (cause: unknown) => void = () => undefined;
    const api = transport({ create: vi.fn(() => new Promise<void>((_resolve, rejectPromise) => { reject = rejectPromise; })) });
    const controller = createSftpController(api);
    await controller.start();
    const creating = controller.create("new", false);
    await controller.dispose();
    reject({ code: "permission_denied" });
    await creating;
    expect(controller.store.getState()).toMatchObject({ attached: null, page: null, busy: false, error: null });
  });
});
