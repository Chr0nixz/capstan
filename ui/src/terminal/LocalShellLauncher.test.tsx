import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { copy } from "../i18n";
import { useWorkspace } from "../workspace";
import { LocalShellLauncher } from "./LocalShellLauncher";

Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
const profiles = [{ id: "powershell51", name: "Windows PowerShell 5.1", is_wsl: false }, { id: "wsl:Ubuntu Dev", name: "WSL: Ubuntu Dev", is_wsl: true }];
const invoke = vi.fn(async () => profiles);
const launch = vi.fn();
let element: HTMLDivElement;
let root: Root;
beforeEach(async () => {
  invoke.mockReset(); invoke.mockResolvedValue(profiles); launch.mockReset();
  element = document.createElement("div"); document.body.append(element); root = createRoot(element);
  Object.defineProperty(window, "__TAURI__", { configurable: true, value: { core: { invoke } } });
  await act(async () => { root.render(<LocalShellLauncher labels={copy.en} onLaunch={launch} />); });
});
afterEach(async () => { await act(async () => root.unmount()); element.remove(); Reflect.deleteProperty(window, "__TAURI__"); vi.restoreAllMocks(); });
async function open(): Promise<void> {
  await act(async () => {
    const details = element.querySelector("details")!; details.open = true; details.dispatchEvent(new Event("toggle"));
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}
const menuItem = (text: string): HTMLButtonElement => [...element.querySelectorAll<HTMLButtonElement>("[role='menuitem']")].find((item) => item.textContent?.includes(text))!;
describe("local shell launcher", () => {
  it("opens a detected shell in one click, with no form in between", async () => {
    await open();
    expect([...element.querySelectorAll("[role='menuitem']")].map((item) => item.textContent)).toEqual([
      `${copy.en.defaultLocalShell}Ctrl+Shift+T`, "Windows PowerShell 5.1", "WSL: Ubuntu Dev", copy.en.customLocalShell,
    ]);
    await act(async () => menuItem("WSL: Ubuntu Dev").click());
    expect(launch).toHaveBeenCalledExactlyOnceWith({ shell_id: "wsl:Ubuntu Dev" }, "WSL: Ubuntu Dev");
    expect(element.querySelector("details")!.open).toBe(false);
  });
  it("discovers shells only when opened and passes the selected WSL ID and Linux directory", async () => {
    expect(invoke).not.toHaveBeenCalled(); await open();
    expect(invoke).toHaveBeenCalledWith("list_local_shells", undefined);
    await act(async () => menuItem(copy.en.customLocalShell).click());
    await act(async () => {
      const select = element.querySelector("select")!; select.value = "wsl:Ubuntu Dev"; select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(element.textContent).toContain("absolute Linux directory");
    await act(async () => {
      const input = element.querySelector("input")!;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, " /tmp ");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => { element.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
    expect(launch).toHaveBeenCalledWith({ shell_id: "wsl:Ubuntu Dev", working_directory: "/tmp" }, "WSL: Ubuntu Dev");
    expect(element.querySelector("details")!.open).toBe(false);
  });
  it("keeps empty options for the default shell and supports Escape without launching", async () => {
    await open();
    await act(async () => menuItem(copy.en.defaultLocalShell).click());
    expect(launch).toHaveBeenCalledWith({}, undefined);
    launch.mockClear(); await open();
    await act(async () => { menuItem(copy.en.defaultLocalShell).dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(element.querySelector("details")!.open).toBe(false); expect(launch).not.toHaveBeenCalled();
  });
  it("shows a retry after discovery failure without removing default-shell access", async () => {
    invoke.mockRejectedValueOnce(new Error("unavailable")); await open();
    expect(element.querySelector('[role="alert"]')?.textContent).toContain("Could not list shells");
    expect(menuItem(copy.en.defaultLocalShell)).toBeDefined();
    await act(async () => { [...element.querySelectorAll("button")].find(button => button.textContent === "Retry")!.click(); });
    expect(element.querySelectorAll("[role='menuitem']")).toHaveLength(4);
  });
  it("moves between shells with the arrow keys", async () => {
    await open();
    const items = [...element.querySelectorAll<HTMLButtonElement>("[role='menuitem']")];
    items[0].focus();
    await act(async () => { items[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true })); });
    expect(document.activeElement).toBe(items[1]);
    await act(async () => { items[1].dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true, cancelable: true })); });
    expect(document.activeElement).toBe(items.at(-1));
  });
  it("does not reset keyboard navigation when the opening animation frame is delayed", async () => {
    const frames: FrameRequestCallback[] = [];
    vi.spyOn(globalThis, "requestAnimationFrame").mockImplementation((callback) => frames.push(callback));
    await open();
    const items = [...element.querySelectorAll<HTMLButtonElement>("[role='menuitem']")];
    items[0].focus();
    await act(async () => { items[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true })); });
    expect(document.activeElement).toBe(items[1]);
    await act(async () => { frames.splice(0).forEach((callback) => callback(16)); });
    expect(document.activeElement).toBe(items[1]);
    expect(launch).not.toHaveBeenCalled();
  });
  it("focuses the default shell on opening and the select when choosing a custom directory", async () => {
    await open();
    expect(document.activeElement).toBe(menuItem(copy.en.defaultLocalShell));
    await act(async () => menuItem(copy.en.customLocalShell).click());
    expect(document.activeElement).toBe(element.querySelector("select"));
    await act(async () => { [...element.querySelectorAll("button")].find((button) => button.textContent === copy.en.backToShells)!.click(); });
    expect(document.activeElement).toBe(menuItem(copy.en.defaultLocalShell));
    expect(launch).not.toHaveBeenCalled();
  });
  it("stores profile options and keeps them when switching or closing another tab", () => {
    const initial = useWorkspace.getState();
    const options = { shell_id: "wsl:Ubuntu Dev", working_directory: "/tmp" };
    initial.addTerminal(options, "WSL: Ubuntu Dev");
    options.working_directory = "/changed";
    const added = useWorkspace.getState().tabs.at(-1)!;
    useWorkspace.getState().selectTab(initial.activeTab);
    expect(useWorkspace.getState().tabs.find(tab => tab.id === added.id)?.launch).toEqual({ kind: "local", options: { shell_id: "wsl:Ubuntu Dev", working_directory: "/tmp" } });
    useWorkspace.getState().closeTerminal(added.id);
  });
});
