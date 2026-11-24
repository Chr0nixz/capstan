import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { copy } from "../i18n";
import { CommandPalette } from "./CommandPalette";
import type { PaletteAction } from "./registry";

Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
const actions: PaletteAction[] = [
  { key: "theme:graphite", action: { id: "set_theme", theme: "graphite", labelKey: "changeTheme", keywordKeys: ["theme"] }, label: "Graphite", keywords: ["theme"] },
  { key: "theme:light", action: { id: "set_theme", theme: "light", labelKey: "changeTheme", keywordKeys: ["theme"] }, label: "Light", keywords: ["theme"] },
];
let container: HTMLDivElement, root: Root;
const onExecute = vi.fn(), onOpenChange = vi.fn();
const render = (open: boolean) => root.render(<StrictMode><CommandPalette open={open} labels={copy.en} actions={actions} onExecute={onExecute} onOpenChange={onOpenChange} /></StrictMode>);
beforeEach(async () => {
  vi.clearAllMocks();
  container = document.createElement("div"); document.body.append(container);
  root = createRoot(container);
  await act(async () => render(true));
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

describe("command palette activation boundaries", () => {
  it("returns focus to where it was when closed without an action", async () => {
    await act(async () => render(false));
    const origin = document.createElement("textarea");
    document.body.append(origin);
    origin.focus();
    await act(async () => render(true));
    expect(document.activeElement).not.toBe(origin);
    await act(async () => render(false));
    // Radix restores focus from a timer after the content unmounts.
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
    expect(document.activeElement).toBe(origin);
    origin.remove();
  });

  it("executes one action before the close commits and accepts a new opening", async () => {
    const option = document.body.querySelector<HTMLButtonElement>(".command-result")!;
    await act(async () => { option.click(); option.click(); });
    expect(onExecute).toHaveBeenCalledExactlyOnceWith(actions[0].action);
    expect(onOpenChange).toHaveBeenCalledExactlyOnceWith(false);
    await act(async () => render(false));
    await act(async () => render(true));
    await act(async () => document.body.querySelector<HTMLButtonElement>(".command-result")!.click());
    expect(onExecute).toHaveBeenCalledTimes(2);
  });

  it.each([{ isComposing: true }, { keyCode: 229 }])("leaves IME Escape to the input method: %j", async (composition) => {
    const search = document.body.querySelector<HTMLInputElement>(".command-search input")!;
    await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", ...composition, bubbles: true, cancelable: true })));
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(onExecute).not.toHaveBeenCalled();
    await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(onOpenChange).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("keeps an empty search from consuming the next valid action", async () => {
    const search = document.body.querySelector<HTMLInputElement>(".command-search input")!;
    const query = async (value: string) => act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, value);
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await query("no such command");
    await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(onExecute).not.toHaveBeenCalled();
    await query("Graphite");
    await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(onExecute).toHaveBeenCalledExactlyOnceWith(actions[0].action);
  });

  it("lists a quick connect target first and keeps it out of the recent list", async () => {
    localStorage.removeItem("capstan.palette-recent");
    const queryAction = (query: string): PaletteAction | null => query.includes("@") ? {
      key: "quick-connect", label: `Connect to ${query}`, keywords: [],
      action: { id: "quick_connect", labelKey: "quickConnect", keywordKeys: [], text: query },
    } : null;
    await act(async () => root.render(<StrictMode><CommandPalette open labels={copy.en} actions={actions} queryAction={queryAction}
      onExecute={onExecute} onOpenChange={onOpenChange} /></StrictMode>));
    const search = document.body.querySelector<HTMLInputElement>(".command-search input")!;
    const type = async (value: string) => act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, value);
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await type("light");
    expect([...document.body.querySelectorAll(".command-result")].map((option) => option.textContent)).toEqual(["Light"]);
    await type("deploy@light");
    const options = [...document.body.querySelectorAll(".command-result")].map((option) => option.textContent);
    expect(options[0]).toBe("Connect to deploy@light");
    await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(onExecute).toHaveBeenCalledExactlyOnceWith({ id: "quick_connect", labelKey: "quickConnect", keywordKeys: [], text: "deploy@light" });
    expect(localStorage.getItem("capstan.palette-recent")).toBeNull();
  });
});
