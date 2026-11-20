import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { copy } from "../i18n";
import { useWorkspace, type TerminalTab } from "../workspace";
import { paneLeaves } from "./tree";
import { TabToolsMenu } from "./TabToolsMenu";

Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });

const labels = copy.en;
let container: HTMLDivElement;
let root: Root;
const activeTab = (): TerminalTab => {
  const state = useWorkspace.getState();
  return state.tabs.find((tab) => tab.id === state.activeTab)!;
};
const item = (name: string): HTMLButtonElement => {
  const found = [...container.querySelectorAll<HTMLButtonElement>("[role^='menuitem']")].find((button) => button.textContent === name);
  expect(found, name).toBeDefined();
  return found!;
};

beforeEach(() => {
  const state = useWorkspace.getState();
  for (const tab of state.tabs) state.closeTerminal(tab.id);
  useWorkspace.getState().addTerminal();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("tab tools menu", () => {
  it("offers the folded pane and sheet keys as menu items", () => {
    const onRequestFocus = vi.fn(), onOpenSnippets = vi.fn(), onOpenTunnels = vi.fn();
    act(() => root.render(<TabToolsMenu tab={activeTab()} labels={labels} onRequestFocus={onRequestFocus} onOpenSnippets={onOpenSnippets} onOpenTunnels={onOpenTunnels} />));
    const button = container.querySelector<HTMLButtonElement>(".tab-tools-more")!;
    expect(button.getAttribute("aria-label")).toBe(labels.moreTools);
    expect(button.getAttribute("aria-haspopup")).toBe("menu");
    expect([...container.querySelectorAll("[role^='menuitem']")].map((element) => element.textContent))
      .toEqual([labels.splitRight, labels.splitDown, labels.broadcastInput, labels.openSnippets, labels.openTunnels]);

    act(() => item(labels.splitRight).click());
    expect(paneLeaves(activeTab().root)).toHaveLength(2);
    expect(onRequestFocus).toHaveBeenCalledTimes(1);
    act(() => item(labels.openSnippets).click());
    act(() => item(labels.openTunnels).click());
    expect(onOpenSnippets).toHaveBeenCalledTimes(1);
    expect(onOpenTunnels).toHaveBeenCalledTimes(1);
  });

  it("keeps broadcast visible on the More key while it is folded away", () => {
    act(() => root.render(<TabToolsMenu tab={activeTab()} labels={labels} onRequestFocus={() => undefined} />));
    expect(item(labels.broadcastInput).getAttribute("aria-checked")).toBe("false");
    act(() => item(labels.broadcastInput).click());
    expect(activeTab().broadcastEnabled).toBe(true);
    act(() => root.render(<TabToolsMenu tab={activeTab()} labels={labels} onRequestFocus={() => undefined} />));
    const button = container.querySelector<HTMLButtonElement>(".tab-tools-more")!;
    expect(button.hasAttribute("data-broadcast")).toBe(true);
    expect(button.getAttribute("aria-label")).toBe(labels.moreToolsBroadcasting);
    expect(item(labels.broadcastInput).getAttribute("aria-checked")).toBe("true");
  });

  it("leaves out sheet keys while the vault is locked and renders nothing without actions", () => {
    act(() => root.render(<TabToolsMenu tab={activeTab()} labels={labels} onRequestFocus={() => undefined} />));
    expect(container.querySelectorAll("[role^='menuitem']")).toHaveLength(3);
    expect(container.querySelector("[role='separator']")).toBeNull();
    act(() => root.render(<TabToolsMenu tab={undefined} labels={labels} onRequestFocus={() => undefined} />));
    expect(container.innerHTML).toBe("");
  });
});
