import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { copy } from "../i18n";
import type { TabId, TerminalLaunch } from "./contracts";
import type { TerminalSessionOptions } from "./session";
import { TerminalPane } from "./TerminalPane";
import { useWorkspace } from "../workspace";

const mocks = vi.hoisted(() => ({
  session: { write: vi.fn(), writeLocal: vi.fn(), setVisible: vi.fn(), setTheme: vi.fn(), focus: vi.fn(), stopInput: vi.fn(), search: vi.fn(() => ({ index: 0, count: 1, match: null, invalidExpression: false })), clearSearch: vi.fn(), dispose: vi.fn(),
    hasSelection: vi.fn(() => false), mouseReporting: vi.fn(() => false), copySelection: vi.fn(), pasteFromClipboard: vi.fn(), clearScreen: vi.fn() },
  controller: { ended: false, sendInput: vi.fn(), acknowledge: vi.fn(), resize: vi.fn(), dispose: vi.fn(async () => undefined) },
  create: vi.fn(),
  options: undefined as TerminalSessionOptions | undefined,
}));
vi.mock("./bridge", () => ({ isTauriAvailable: () => true }));
vi.mock("./session", () => ({
  createTerminalSession: (_container: HTMLElement, _visible: boolean, options: TerminalSessionOptions) => {
    mocks.options = options;
    return mocks.session;
  },
}));
vi.mock("../connections/controller", () => ({ createRecoveringController: (...args: unknown[]) => { mocks.create(...args); return mocks.controller; } }));

const launch: TerminalLaunch = { kind: "local", options: {} };
const tabId = "tab-test" as TabId;

describe("terminal view ownership", () => {
  beforeEach(() => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    vi.clearAllMocks();
    mocks.controller.ended = false;
  });

  it("leaves reported right clicks to xterm and opens the local menu for Shift or keyboard", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible />); });
    const host = container.querySelector<HTMLDivElement>(".terminal-host")!;
    const menu = container.querySelector<HTMLDivElement>(".terminal-menu")!;
    const toggle = (newState: string): void => { menu.dispatchEvent(Object.assign(new Event("toggle"), { newState })); };
    let open = false;
    menu.showPopover = vi.fn(() => { open = true; toggle("open"); });
    menu.hidePopover = vi.fn(() => { open = false; toggle("closed"); });
    menu.matches = vi.fn((selector) => selector === ":popover-open" && open);
    const mouse = (shiftKey = false): MouseEvent => new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2, shiftKey });
    mocks.session.mouseReporting.mockReturnValue(true);
    await act(async () => { host.dispatchEvent(mouse()); });
    expect(menu.showPopover).not.toHaveBeenCalled();
    await act(async () => { host.dispatchEvent(mouse(true)); });
    expect(menu.showPopover).toHaveBeenCalledOnce();
    const [copyItem, paste, clear] = [...menu.querySelectorAll("button")];
    expect(copyItem.disabled).toBe(true);
    expect(document.activeElement).toBe(paste);
    await act(async () => { paste.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })); });
    expect(document.activeElement).toBe(clear);
    const focuses = mocks.session.focus.mock.calls.length;
    await act(async () => { clear.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect(open).toBe(false);
    expect(mocks.session.focus.mock.calls.length).toBeGreaterThan(focuses);
    mocks.session.hasSelection.mockReturnValue(true);
    await act(async () => { host.dispatchEvent(new KeyboardEvent("keydown", { key: "F10", shiftKey: true, bubbles: true, cancelable: true })); });
    expect(document.activeElement).toBe(copyItem);
    await act(async () => { copyItem.click(); });
    expect(mocks.session.copySelection).toHaveBeenCalledOnce();
    mocks.session.mouseReporting.mockReturnValue(false);
    await act(async () => { host.dispatchEvent(mouse()); clear.click(); });
    expect(mocks.session.clearScreen).toHaveBeenCalledOnce();
    await act(async () => root.unmount());
    container.remove();
  });

  it("does not restart the backend when labels or visibility change", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible />); });
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy["zh-CN"]} visible={false} />); });
    expect(mocks.create).toHaveBeenCalledTimes(1);
    expect(mocks.controller.dispose).not.toHaveBeenCalled();
    expect(mocks.session.setVisible).toHaveBeenLastCalledWith(false);
    await act(async () => { root.unmount(); });
    expect(mocks.controller.dispose).toHaveBeenCalledTimes(1);
    expect(mocks.session.dispose).toHaveBeenCalledTimes(1);
  });

  it("focuses the terminal when it becomes visible or focus is requested", async () => {
    const root = createRoot(document.createElement("div"));
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible={false} focusRequest={0} />); });
    expect(mocks.session.focus).not.toHaveBeenCalled();
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible focusRequest={0} />); });
    expect(mocks.session.focus).toHaveBeenCalledTimes(1);
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible focusRequest={1} />); });
    expect(mocks.session.focus).toHaveBeenCalledTimes(2);
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible={false} focusRequest={2} />); });
    expect(mocks.session.focus).toHaveBeenCalledTimes(2);
    await act(async () => { root.unmount(); });
  });

  it("ignores input errors after the process ended instead of reporting a delivery failure", async () => {
    const root = createRoot(document.createElement("div"));
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible />); });
    mocks.controller.ended = true;
    mocks.options?.onInputError?.(new Error("session_closed"));
    expect(mocks.session.writeLocal).not.toHaveBeenCalled();
    expect(mocks.controller.dispose).not.toHaveBeenCalled();
    mocks.controller.ended = false;
    mocks.options?.onInputError?.(new Error("delivery uncertain"));
    expect(mocks.session.writeLocal).toHaveBeenCalledTimes(1);
    expect(mocks.controller.dispose).toHaveBeenCalledTimes(1);
    await act(async () => { root.unmount(); });
  });

  it("opens terminal search from Ctrl+Shift+F and restores terminal focus on close", async () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible />); });
    const pane = container.querySelector(".terminal-pane-root");
    expect(pane).not.toBeNull();
    await act(async () => {
      pane?.dispatchEvent(new KeyboardEvent("keydown", { key: "f", ctrlKey: true, shiftKey: true, bubbles: true }));
    });
    expect(container.querySelector(".terminal-search-toolbar")).not.toBeNull();
    const close = container.querySelector<HTMLButtonElement>(`button[aria-label="${copy.en.closeSearch}"]`);
    await act(async () => { close?.click(); });
    expect(mocks.session.clearSearch).toHaveBeenCalledTimes(1);
    expect(container.querySelector(".terminal-search-toolbar")).toBeNull();
    await act(async () => { root.unmount(); });
  });

  it("offers an in-place restart once the session has ended, also on Enter", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const restart = vi.fn();
    useWorkspace.getState().addTerminal();
    const tab = useWorkspace.getState().tabs.at(-1)!;
    useWorkspace.setState({ tabs: [{ ...tab, id: tabId, root: { kind: "pane", paneId: tab.focusedPaneId, launch }, launch, paneStates: { [tab.focusedPaneId]: { failed: true } } }], activeTab: tabId });
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible onRestart={restart} onClose={() => undefined} />); });
    expect(container.querySelector(".session-ended")?.textContent).toContain("Session failed");
    const restartButton = [...container.querySelectorAll("button")].find((button) => button.textContent === "Restart shell");
    await act(async () => restartButton?.click());
    expect(restart).toHaveBeenCalledTimes(1);
    await act(async () => container.querySelector(".terminal-host")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(restart).toHaveBeenCalledTimes(2);
    await act(async () => { root.unmount(); });
    container.remove();
  });

  it("only selects Reconnect when Enter is pressed after a production session ends", async () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const restart = vi.fn();
    useWorkspace.getState().addTerminal();
    const tab = useWorkspace.getState().tabs.at(-1)!;
    useWorkspace.setState({ tabs: [{ ...tab, id: tabId, root: { kind: "pane", paneId: tab.focusedPaneId, launch }, launch, paneStates: { [tab.focusedPaneId]: { failed: true } } }], activeTab: tabId });
    await act(async () => { root.render(<TerminalPane tabId={tabId} launch={launch} labels={copy.en} visible production onRestart={restart} onClose={() => undefined} />); });
    expect(container.querySelector(".session-ended .prod-chip")?.textContent).toBe("Production");
    await act(async () => container.querySelector(".terminal-host")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(restart).not.toHaveBeenCalled();
    expect(document.activeElement?.textContent).toBe("Restart shell");
    await act(async () => { root.unmount(); });
    container.remove();
  });
});
