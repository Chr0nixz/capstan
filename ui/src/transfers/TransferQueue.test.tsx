import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-transfer.json";
import { TransferQueue } from "./TransferQueue";
import type { SftpSessionId } from "../sftp/contracts";
import type { HostId, TerminalSessionId } from "../terminal/contracts";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listener: undefined as ((payload?: { error?: string } | null) => void) | undefined }));
vi.mock("../terminal/bridge", () => ({
  isTauriAvailable: () => true,
  invokeTauri: (command: string, args?: unknown) => mocks.invoke(command, args),
  listenTauriEvent: async (_name: string, callback: (payload?: { error?: string } | null) => void) => { mocks.listener = callback; return () => { mocks.listener = undefined; }; },
}));
let root: Root, container: HTMLDivElement;
const button = (name: string): HTMLButtonElement | undefined => [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === name);
beforeEach(async () => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation(async (command: string) => command === "transfer_snapshot" ? fixture : command === "transfer_recovery_snapshot" ? { entries: [], incomplete: false } : undefined);
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  await act(async () => { root.render(<TransferQueue locale="en" hosts={{ [fixture[0].hostId]: "Build server" }} />); });
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); });
const batchTask = { ...fixture[0], batch: { id: "00000000-0000-4000-8000-000000000005", name: "Config folder", files: 5, conflicts: 4 }, conflict: { ...fixture[0].conflict, replaceAllowed: true, batchEligible: true } };
async function showBatch(locale: "en" | "zh-CN" = "en"): Promise<void> {
  mocks.invoke.mockImplementation(async (command: string) => command === "transfer_snapshot" ? [batchTask] : command === "transfer_recovery_snapshot" ? { entries: [], incomplete: false } : undefined);
  await act(async () => root.unmount()); root = createRoot(container);
  await act(async () => root.render(<TransferQueue locale={locale} hosts={{}} />));
  await act(async () => button(locale === "en" ? "Review destination" : "检查传输目标")?.click());
}
describe("global transfer entry", () => {
  it.each(["en", "zh-CN"] as const)("offers cleanup without retry or resume for an owned nonresumable partial in %s", async (locale) => {
    const item = { id: fixture[0].id, hostId: fixture[0].hostId, name: "report.bin", remotePath: "/drop/report.bin", direction: "upload", total: "10", partialSize: "4", localPartial: false, state: "cleanup_required", updatedSeconds: "1" };
    const task = { ...fixture[0], conflict: null, state: "failed", cleanupRequired: true };
    const terminal = "11111111-1111-4111-8111-111111111111" as TerminalSessionId, session = "22222222-2222-4222-8222-222222222222" as SftpSessionId;
    let partialPresent = true;
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "cleanup_recovered_transfer_upload") { partialPresent = false; return undefined; }
      return command === "transfer_snapshot" ? [task] : command === "transfer_recovery_snapshot" ? { entries: partialPresent ? [item] : [], incomplete: false } : command === "native_export_snapshot" ? [] : undefined;
    });
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => root.render(<TransferQueue locale={locale} hosts={{}} sessions={[{ terminal, session, hostId: fixture[0].hostId as HostId }]} />));
    expect(container.textContent).toContain(locale === "en" ? "Cleanup required; cannot resume" : "需要清理，无法续传");
    for (const name of ["Resume upload", "Resume download", "Retry", "继续上传", "继续下载", "重试"]) expect(button(name)).toBeUndefined();
    expect(button(locale === "en" ? "Remove partial" : "删除残留")).toBeDefined();
    expect(mocks.invoke.mock.calls.some(([command]) => String(command).startsWith("cleanup_recovered"))).toBe(false);
    await act(async () => button(locale === "en" ? "Remove partial" : "删除残留")?.click());
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
    await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((entry) => entry.textContent === (locale === "en" ? "Remove partial" : "删除残留"))?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("cleanup_recovered_transfer_upload", { terminal, session, id: item.id });
    expect(container.querySelector(".transfer-recovery")).toBeNull();
    expect(button(locale === "en" ? "Retry" : "重试")).toBeUndefined();
  });
  it("summarizes plain progress while folded and opens with a red badge for a decision", async () => {
    const running = { ...fixture[0], conflict: null, state: "running" };
    mocks.invoke.mockImplementation(async (command: string) => command === "transfer_snapshot" ? [running] : command === "transfer_recovery_snapshot" ? { entries: [], incomplete: false } : undefined);
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => { root.render(<TransferQueue locale="en" hosts={{}} />); await new Promise((resolve) => setTimeout(resolve, 0)); });
    const heading = container.querySelector(".transfer-queue-heading");
    expect(heading?.getAttribute("aria-expanded")).toBe("false");
    expect(heading?.textContent).toBe("Transfers · 1 · 1 in progress");
    mocks.invoke.mockImplementation(async (command: string) => command === "transfer_snapshot" ? [{ ...running, state: "failed", version: String(BigInt(running.version) + 1n) }] : command === "transfer_recovery_snapshot" ? { entries: [], incomplete: false } : undefined);
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 300)); });
    expect(container.querySelector(".transfer-queue-heading")?.getAttribute("aria-expanded")).toBe("true");
    expect(container.querySelector(".count-badge")?.getAttribute("data-attention")).toBe("true");
    expect(container.querySelector(".transfer-summary")?.textContent).toBe("1 needs a decision");
  });
  it("reports paused transfers separately from running ones", async () => {
    const running = { ...fixture[0], conflict: null, state: "running" };
    const paused = { ...fixture[0], id: "00000000-0000-4000-8000-000000000009", conflict: null, state: "paused" };
    const snapshot = (tasks: unknown[]) => async (command: string) => command === "transfer_snapshot" ? tasks
      : command === "transfer_recovery_snapshot" ? { entries: [], incomplete: false } : undefined;
    mocks.invoke.mockImplementation(snapshot([running, paused]));
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => { root.render(<TransferQueue locale="en" hosts={{}} />); await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(container.querySelector(".count-badge")?.textContent).toBe("2");
    expect(container.querySelector(".transfer-summary")?.textContent).toBe("1 in progress · 1 paused");
    mocks.invoke.mockImplementation(snapshot([paused]));
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => { root.render(<TransferQueue locale="zh-CN" hosts={{}} />); await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(container.querySelector(".transfer-queue")?.getAttribute("data-active")).toBeNull();
    expect(container.querySelector(".transfer-summary")?.textContent).toBe("1 个已暂停");
  });

  it("clears finished rows from view and marks production hosts", async () => {
    const done = { ...fixture[0], conflict: null, state: "completed" };
    mocks.invoke.mockImplementation(async (command: string) => command === "transfer_snapshot" ? [done] : command === "transfer_recovery_snapshot" ? { entries: [], incomplete: false } : undefined);
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => { root.render(<TransferQueue locale="en" hosts={{ [done.hostId]: "prod-api" }} production={new Set([done.hostId])} />); await new Promise((resolve) => setTimeout(resolve, 0)); });
    await act(async () => container.querySelector<HTMLButtonElement>(".transfer-queue-heading")?.click());
    expect(container.querySelector(".transfer-row .prod-chip")?.textContent).toBe("Production");
    await act(async () => button("Clear finished")?.click());
    expect(container.querySelector(".transfer-row")).toBeNull();
    expect(container.textContent).toContain("No transfers yet.");
  });
  it.each(["en", "zh-CN"] as const)("keeps unverifiable old records for manual inspection without unsafe actions in %s", async (locale) => {
    const recovery = { entries: [
      { id: fixture[0].id, hostId: fixture[0].hostId, name: "old-download.bin", remotePath: "/drop/old-download.bin", direction: "download", total: "10", partialSize: "4", localPartial: true, state: "manual_inspection", updatedSeconds: "1" },
      { id: "33333333-3333-4333-8333-333333333333", hostId: fixture[0].hostId, name: "old-upload.bin", remotePath: "/drop/old-upload.bin", direction: "upload", total: "10", partialSize: "4", localPartial: false, state: "manual_inspection", updatedSeconds: "1" },
    ], incomplete: false };
    mocks.invoke.mockImplementation(async (command: string) => command === "transfer_recovery_snapshot" ? recovery : command === "transfer_snapshot" || command === "native_export_snapshot" ? [] : undefined);
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => root.render(<TransferQueue locale={locale} hosts={{}} sessions={[{
      terminal: "11111111-1111-4111-8111-111111111111" as TerminalSessionId,
      session: "22222222-2222-4222-8222-222222222222" as SftpSessionId,
      hostId: fixture[0].hostId as HostId,
    }]} />));
    expect(container.textContent).toContain(locale === "en" ? "Manual inspection required" : "需人工检查");
    expect(container.textContent).toContain(locale === "en" ? "Keep the partial" : "请保留残留");
    for (const label of ["Resume upload", "Resume download", "Remove partial", "继续上传", "继续下载", "删除残留"]) expect(button(label)).toBeUndefined();
    expect(mocks.invoke.mock.calls.some(([command]) => String(command).startsWith("select_recovered") || String(command).startsWith("retry_recovered") || String(command).startsWith("cleanup_recovered"))).toBe(false);
  });
  it("resumes a discovered download only through a fresh destination picker", async () => {
    const recovery = { entries: [{ id: fixture[0].id, hostId: fixture[0].hostId, name: "report.bin", remotePath: "/drop/report.bin", direction: "download", total: "10", partialSize: "4", localPartial: true, state: "available", updatedSeconds: "1" }], incomplete: false };
    const terminal = "11111111-1111-4111-8111-111111111111" as TerminalSessionId, session = "22222222-2222-4222-8222-222222222222" as SftpSessionId;
    const selection = "33333333-3333-4333-8333-333333333333";
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "transfer_snapshot" || command === "native_export_snapshot") return [];
      if (command === "transfer_recovery_snapshot") return recovery;
      if (command === "select_recovered_transfer_destination") return { token: selection, name: "report.bin" };
      return undefined;
    });
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => root.render(<TransferQueue locale="en" hosts={{}} sessions={[{ terminal, session, hostId: fixture[0].hostId as HostId }]} />));
    await act(async () => button("Resume download")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("select_recovered_transfer_destination", { terminal, session, id: fixture[0].id, locale: "en" });
    expect(mocks.invoke).toHaveBeenCalledWith("retry_recovered_transfer_download", { terminal, session, id: fixture[0].id, selection });
    expect(mocks.invoke.mock.calls.some(([command]) => command === "cleanup_recovered_transfer_download")).toBe(false);
  });
  it("requires explicit folder consent and sends one batch decision bound to the displayed revision", async () => {
    await showBatch();
    const checkbox = document.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    expect(checkbox.checked).toBe(false);
    expect(document.body.textContent).toContain("Config folder · 5 files");
    expect(document.body.textContent).toContain("4 existing destinations");
    expect(document.body.textContent).toContain("Changed destinations need another decision");
    expect(button("Replace batch conflicts")).toBeUndefined();
    await act(async () => checkbox.click());
    await act(async () => button("Replace batch conflicts")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("resolve_transfer_conflict", { id: batchTask.id, revision: batchTask.conflict.revision, decision: { kind: "replace_batch" } });
  });
  it("offers a localized batch skip while save-as remains an individual decision", async () => {
    await showBatch("zh-CN");
    expect(document.body.textContent).toContain("预检到的 4 个同名目标");
    await act(async () => document.querySelector<HTMLInputElement>('input[type="checkbox"]')?.click());
    await act(async () => button("跳过本批次冲突")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("resolve_transfer_conflict", { id: batchTask.id, revision: batchTask.conflict.revision, decision: { kind: "skip_batch" } });
    await act(async () => button("另存为")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("resolve_transfer_conflict", { id: batchTask.id, revision: batchTask.conflict.revision, decision: { kind: "rename", name: batchTask.name } });
  });
  it("clears batch consent when a later target revision needs another decision", async () => {
    vi.useFakeTimers();
    await showBatch();
    await act(async () => document.querySelector<HTMLInputElement>('input[type="checkbox"]')?.click());
    expect(button("Replace batch conflicts")).toBeDefined();
    const changed = { ...batchTask, version: "9007199254740995", conflict: { ...batchTask.conflict, revision: "9007199254740995" } };
  mocks.invoke.mockImplementation(async (command: string) => command === "transfer_snapshot" ? [changed] : command === "transfer_recovery_snapshot" ? { entries: [], incomplete: false } : undefined);
    await act(async () => vi.advanceTimersByTimeAsync(250));
    expect(document.querySelector<HTMLInputElement>('input[type="checkbox"]')?.checked).toBe(false);
    expect(button("Replace batch conflicts")).toBeUndefined();
    await act(async () => button("Replace")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("resolve_transfer_conflict", { id: changed.id, revision: changed.conflict.revision, decision: { kind: "replace" } });
  });
  it("keeps editor cleanup decisions separate from canceling transfers on exit", async () => {
    await act(async () => mocks.listener?.({ error: "editor_residual" }));
    expect(document.body.textContent).toContain("Review the open or recovered editor copies before exiting");
    await act(async () => button("Keep open")?.click());
    expect(mocks.invoke.mock.calls.some(([command]) => command === "confirm_transfer_exit" || command === "close_remote_edit")).toBe(false);
  });
  it("identifies a preference drain failure without reporting a remote operation failure", async () => {
    await act(async () => mocks.listener?.({ error: "settings_unavailable" }));
    expect(document.body.textContent).toContain("Panel preferences did not finish saving");
    expect(button("Retry exit")).toBeDefined();
    mocks.invoke.mockRejectedValueOnce({ code: "settings_unavailable" });
    await act(async () => button("Retry exit")?.click());
    expect(document.querySelector('[role="alert"]')?.textContent).toContain("Panel preferences could not be saved");
    expect(document.body.textContent).not.toContain("remote operation failed");
  });
  it("identifies the host and destination, and offers save-as when replacement is unavailable", async () => {
    expect(container.textContent).toContain("Build server");
    expect(container.textContent).toContain("中文 file.bin");
    await act(async () => button("Review destination")?.click());
    expect(button("Replace")?.disabled).toBe(true);
    expect(document.body.textContent).toContain("cannot safely replace");
    expect(button("Save as")?.disabled).toBe(false);
    await act(async () => button("Skip")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("resolve_transfer_conflict", { id: fixture[0].id, revision: fixture[0].conflict.revision, decision: { kind: "skip" } });
  });
  it("keeps the window open by default and only exits after the chosen action", async () => {
    await act(async () => mocks.listener?.());
    expect(document.body.textContent).toContain("File operations are still active");
    expect(document.body.textContent).toContain("Current removals and final commits finish before exit");
    await act(async () => button("Keep open")?.click());
    expect(mocks.invoke.mock.calls.some(([command]) => command === "confirm_transfer_exit")).toBe(false);
    await act(async () => mocks.listener?.());
    await act(async () => button("Cancel remaining operations and exit")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("confirm_transfer_exit", undefined);
  });
  it("renders the queue and conflict actions in Chinese", async () => {
    await act(async () => root.render(<TransferQueue locale="zh-CN" theme="light" hosts={{}} />));
    expect(container.textContent).toContain("传输任务");
    expect(container.querySelector(".transfer-queue")?.getAttribute("data-theme")).toBe("light");
    await act(async () => button("检查传输目标")?.click());
    expect(document.querySelector('[role="dialog"]')?.getAttribute("data-theme")).toBe("light");
    expect(button("覆盖")?.disabled).toBe(true);
    expect(button("另存为")).toBeDefined();
  });
  it("guides an interrupted transfer to a matching SSH session", async () => {
    const interrupted = [{ ...fixture[0], state: "interrupted", conflict: null, error: "connection_lost" }];
    mocks.invoke.mockImplementation(async (command: string) => command === "transfer_snapshot" ? interrupted : command === "transfer_recovery_snapshot" ? { entries: [], incomplete: false } : undefined);
    const focusHost = vi.fn();
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<TransferQueue locale="en" hosts={{ [fixture[0].hostId]: "Build server" }} onRetrySessionRequested={focusHost} />));
    await act(async () => button("Retry")?.click());
    expect(document.body.textContent).toContain("Reconnect the SSH session for this host");
    expect(focusHost).toHaveBeenCalledWith(fixture[0].hostId);
    expect(button("Open SSH session")).toBeDefined();
  });
  it("automatically retries after the requested host receives a new SFTP session", async () => {
    const interrupted = [{ ...fixture[0], direction: "download", state: "interrupted", conflict: null, error: "connection_lost" }];
    mocks.invoke.mockImplementation(async (command: string) => command === "transfer_snapshot" ? interrupted : command === "transfer_recovery_snapshot" ? { entries: [], incomplete: false } : undefined);
    const focusHost = vi.fn();
    const hostId = fixture[0].hostId as HostId;
    const terminal = "11111111-1111-4111-8111-111111111111" as TerminalSessionId;
    const session = "22222222-2222-4222-8222-222222222222" as SftpSessionId;
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<TransferQueue locale="en" hosts={{ [hostId]: "Build server" }} onRetrySessionRequested={focusHost} />));
    await act(async () => button("Retry")?.click());
    expect(focusHost).toHaveBeenCalledWith(hostId);
    await act(async () => root.render(<TransferQueue locale="en" hosts={{ [hostId]: "Build server" }} sessions={[{ terminal, hostId, session }]} onRetrySessionRequested={focusHost} />));
    expect(mocks.invoke).toHaveBeenCalledWith("retry_transfer_on_session", { id: fixture[0].id, terminal, session });
  });
  it.each(["en", "zh-CN"] as const)("keeps source-verified upload resume without unverified removal in %s", async (locale) => {
    const recovery = { entries: [{ id: fixture[0].id, hostId: fixture[0].hostId, name: "report.bin", remotePath: "/drop/report.bin",
      direction: "upload", total: "10", partialSize: null, localPartial: false, state: "awaiting_reconnect", updatedSeconds: "1" }], incomplete: false };
    const terminal = "11111111-1111-4111-8111-111111111111" as TerminalSessionId;
    const session = "22222222-2222-4222-8222-222222222222" as SftpSessionId;
    const selection = "33333333-3333-4333-8333-333333333333";
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "transfer_snapshot") return [];
      if (command === "transfer_recovery_snapshot") return recovery;
      if (command === "select_transfer_source") return { token: selection, name: "report.bin" };
      return undefined;
    });
    await act(async () => root.unmount()); root = createRoot(container);
    await act(async () => root.render(<TransferQueue locale={locale} hosts={{ [fixture[0].hostId]: "Build server" }} sessions={[{ terminal, hostId: fixture[0].hostId as HostId, session }]} />));
    expect(container.textContent).toContain(locale === "en" ? "Interrupted transfer files" : "中断传输残留");
    expect(container.textContent).toContain(locale === "en" ? "Reconnect to verify" : "重新连接后核验");
    expect(container.textContent).toContain(locale === "en" ? "Ownership cannot be verified" : "无法核验残留归属");
    for (const label of ["Remove partial", "删除残留"]) expect(button(label)).toBeUndefined();
    await act(async () => button(locale === "en" ? "Resume upload" : "继续上传")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("select_transfer_source", { terminal, session, parent: "/drop", locale });
    expect(mocks.invoke).toHaveBeenCalledWith("retry_recovered_transfer_upload", { id: fixture[0].id, terminal, session, selection });
    expect(mocks.invoke.mock.calls.some(([command]) => String(command).startsWith("cleanup_recovered"))).toBe(false);
  });
});
