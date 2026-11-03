import { describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-transfer.json";
import { decodeTransfers, type Transfer } from "./contracts";
import { createTransferController } from "./controller";
import type { SftpSessionId } from "../sftp/contracts";
import type { TerminalSessionId } from "../terminal/contracts";
function setup(tasks: Transfer[] = decodeTransfers(fixture)) {
  const transport = { snapshot: vi.fn().mockResolvedValue(tasks), recoverySnapshot: vi.fn().mockResolvedValue({ entries: [], incomplete: false }), cancel: vi.fn().mockResolvedValue(undefined), resolve: vi.fn().mockResolvedValue(undefined), exit: vi.fn().mockResolvedValue(undefined), retryOnSession: vi.fn().mockResolvedValue(undefined), retryWithSelection: vi.fn().mockResolvedValue(undefined), retryRecoveredUpload: vi.fn().mockResolvedValue(undefined) };
  return { transport, controller: createTransferController(transport), tasks };
}
describe("transfer queue ownership and late responses", () => {
  it.each(["cleanup_required", "manual_inspection"] as const)("keeps %s records actionable beside their final current-process task", async (state) => {
    const task = { ...decodeTransfers(fixture)[0], state: "failed" as const, conflict: null, cleanupRequired: true };
    const { transport, controller } = setup([task]);
    const recovery = { id: task.id, hostId: task.hostId, name: task.name, remotePath: task.remotePath, direction: task.direction,
      total: task.total, partialSize: null, localPartial: false, state, updatedSeconds: "1" };
    transport.recoverySnapshot.mockResolvedValueOnce({ entries: [recovery], incomplete: false });
    await controller.refresh();
    expect(controller.store.getState().recovery).toEqual([recovery]);
    expect(controller.store.getState().tasks).toEqual([task]);
  });
  it("serializes polling and rejects responses after disposal", async () => {
    const { transport, controller, tasks } = setup();
    let finish: (tasks: Transfer[]) => void = () => undefined;
    transport.snapshot.mockImplementationOnce(() => new Promise<Transfer[]>((resolve) => { finish = resolve; }));
    const first = controller.refresh(); await controller.refresh();
    expect(transport.snapshot).toHaveBeenCalledTimes(1);
    controller.dispose(); finish(tasks); await first;
    expect(controller.store.getState().tasks).toEqual([]);
  });
  it("keeps final results against newer progress and row identity against reordering", async () => {
    const { transport, controller, tasks } = setup();
    const complete = { ...tasks[0], state: "completed" as const, conflict: null };
    transport.snapshot.mockResolvedValueOnce([complete]); await controller.refresh();
    transport.snapshot.mockResolvedValueOnce([{ ...tasks[0], version: "9007199254740999" }]); await controller.refresh();
    expect(controller.store.getState().tasks[0].state).toBe("completed");
    const second = { ...complete, id: "00000000-0000-4000-8000-000000000003" };
    transport.snapshot.mockResolvedValueOnce([second, complete]); await controller.refresh();
    expect(controller.store.getState().tasks.map((task) => task.id)).toEqual([complete.id, second.id]);
  });
  it("uses the displayed conflict revision exactly once while a decision is pending", async () => {
    const { transport, controller, tasks } = setup();
    let finish: () => void = () => undefined;
    transport.resolve.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    const first = controller.resolve(tasks[0], { kind: "skip" });
    await controller.resolve(tasks[0], { kind: "replace" });
    expect(transport.resolve).toHaveBeenCalledTimes(1);
    expect(transport.resolve).toHaveBeenCalledWith(tasks[0].id, tasks[0].conflict?.revision, { kind: "skip" });
    finish(); await first;
  });
  it("reopens an explicitly retried row and rejects its delayed old final snapshot", async () => {
    const task = { ...decodeTransfers(fixture)[0], state: "interrupted" as const, conflict: null };
    const { transport, controller } = setup([task]);
    await controller.refresh();
    await controller.retryOnSession(task.id, "terminal" as TerminalSessionId, "sftp" as SftpSessionId);
    expect(controller.store.getState().tasks[0].state).toBe("retrying");
    const running = { ...task, state: "running" as const, version: (BigInt(task.version) + 1n).toString() as Transfer["version"] };
    transport.snapshot.mockResolvedValueOnce([running]); await controller.refresh();
    expect(controller.store.getState().tasks[0].state).toBe("running");
    transport.snapshot.mockResolvedValueOnce([task]); await controller.refresh();
    expect(controller.store.getState().tasks[0]).toEqual(running);
  });
  it("keeps a rejected retry final and excludes current-process jobs from startup recovery", async () => {
    const task = { ...decodeTransfers(fixture)[0], state: "interrupted" as const, conflict: null };
    const { transport, controller } = setup([task]);
    transport.recoverySnapshot.mockResolvedValueOnce({ entries: [{ id: task.id }], incomplete: false });
    await controller.refresh();
    expect(controller.store.getState().recovery).toEqual([]);
    transport.retryOnSession.mockRejectedValueOnce({ code: "scope_expired" });
    await expect(controller.retryOnSession(task.id, "terminal" as TerminalSessionId, "sftp" as SftpSessionId)).rejects.toEqual({ code: "scope_expired" });
    expect(controller.store.getState().tasks[0].state).toBe("interrupted");
  });
  it("preserves action errors across the snapshot refresh", async () => {
    const { transport, controller, tasks } = setup();
    transport.cancel.mockRejectedValueOnce({ code: "permission_denied" });
    await controller.cancel(tasks[0].id);
    expect(controller.store.getState().error).toBe("permission_denied");
  });
  it("binds an interrupted retry to the explicitly selected session and source", async () => {
    const { transport, controller, tasks } = setup([{ ...decodeTransfers(fixture)[0], state: "interrupted", conflict: null }]);
    const terminal = "terminal" as TerminalSessionId, session = "sftp" as SftpSessionId;
    await controller.retryWithSelection(tasks[0].id, terminal, session, "selection");
    expect(transport.retryWithSelection).toHaveBeenCalledWith(tasks[0].id, terminal, session, "selection");
  });
  it("refreshes discovered residuals with the queue and binds recovered upload to a fresh source", async () => {
    const { transport, controller } = setup([]);
    const task = decodeTransfers(fixture)[0];
    const recovery = { id: task.id, hostId: task.hostId, name: "report.bin", remotePath: "/drop/report.bin", direction: "upload" as const,
      total: "10" as Transfer["total"], partialSize: null, localPartial: false, state: "awaiting_reconnect" as const, updatedSeconds: "1" as Transfer["total"] };
    transport.recoverySnapshot.mockResolvedValueOnce({ entries: [recovery], incomplete: true });
    await controller.refresh();
    expect(controller.store.getState().recovery).toEqual([recovery]);
    expect(controller.store.getState().recoveryIncomplete).toBe(true);
    const terminal = "terminal" as TerminalSessionId, session = "sftp" as SftpSessionId;
    await controller.retryRecoveredUpload(recovery.id, terminal, session, "fresh-selection");
    expect(transport.retryRecoveredUpload).toHaveBeenCalledWith(recovery.id, terminal, session, "fresh-selection");
  });
});
