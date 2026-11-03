import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-delete.json";
import type { TerminalSessionId } from "../terminal/contracts";
import type { RemoteEntryId, SftpSessionId } from "./contracts";
import { TreeDeleteDialog } from "./TreeDeleteDialog";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../terminal/bridge", () => ({ invokeTauri: (command: string, args?: unknown) => mocks.invoke(command, args) }));
async function mount(locale: "en" | "zh-CN" = "en") {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container), close = vi.fn(), changed = vi.fn();
  await act(async () => root.render(<TreeDeleteDialog terminal={"terminal" as TerminalSessionId} session={"session" as SftpSessionId} entry={"entry" as RemoteEntryId} hostLabel="Build server" locale={locale} theme="light" onClose={close} onChanged={changed} />));
  const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === label);
  return { close, changed, button, cleanup: async () => { await act(async () => root.unmount()); container.remove(); } };
}
describe("recursive delete confirmation and results", () => {
  it("confirms the host/path/counts once and shows individual partial failures", async () => {
    mocks.invoke.mockReset().mockImplementation(async (command: string) => command === "prepare_remote_tree_delete" ? fixture.summary : command === "apply_remote_tree_delete" ? fixture.report : undefined);
    const view = await mount();
    try {
      expect(document.body.textContent).toContain("/files/tree");
      expect(document.body.textContent).toContain("Build server");
      expect(mocks.invoke.mock.calls.some(([command]) => command === "apply_remote_tree_delete")).toBe(false);
      await act(async () => view.button("Delete this tree")?.click());
      expect(document.body.textContent).toContain("1 removed · 2 failed or retained");
      expect(document.body.textContent).toContain("Permission denied");
      expect(view.button("Delete this tree")).toBeUndefined();
      expect(view.changed).toHaveBeenCalledTimes(1);
    } finally { await view.cleanup(); }
  });
  it("cancels the remaining items while waiting for the admitted removal result", async () => {
    let finish: (report: unknown) => void = () => undefined;
    mocks.invoke.mockReset().mockImplementation(async (command: string) => command === "prepare_remote_tree_delete" ? fixture.summary : command === "apply_remote_tree_delete" ? new Promise((resolve) => { finish = resolve; }) : undefined);
    const view = await mount("zh-CN");
    try {
      await act(async () => view.button("删除此目录树")?.click());
      await act(async () => view.button("取消剩余删除")?.click());
      expect(mocks.invoke).toHaveBeenCalledWith("cancel_remote_tree_delete", { terminal: "terminal", session: "session", token: fixture.summary.token });
      expect(view.close).not.toHaveBeenCalled();
      expect(document.body.textContent).toContain("正在等待当前删除结果");
      await act(async () => finish(fixture.report));
      expect(document.body.textContent).toContain("已删除 1 项");
      await act(async () => view.button("关闭")?.click());
      expect(view.close).toHaveBeenCalledTimes(1);
    } finally { await view.cleanup(); }
  });
});
