import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { QuakeSettings } from "./QuakeSettings";
import { desktopDefaults, useDesktopPreferences } from "../themes/store";
import { copy } from "../i18n";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../terminal/bridge", () => ({ invokeTauri: invoke }));
Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
let root: Root;
let container: HTMLDivElement;
beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue({ enabled: false, errorCode: null });
  useDesktopPreferences.setState({ preferences: desktopDefaults, ready: true, saving: false });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

async function setHotkey(value: string): Promise<void> {
  await act(async () => {
    const input = container.querySelector<HTMLInputElement>("input.mono")!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it("has no default hotkey and saves only the explicit Quake settings", async () => {
  await act(async () => root.render(<QuakeSettings locale="en" />));
  expect(container.querySelector<HTMLInputElement>("input[type='checkbox']")!.checked).toBe(false);
  expect(container.querySelector<HTMLInputElement>("input.mono")!.value).toBe("");
  await act(async () => container.querySelector<HTMLInputElement>("input[type='checkbox']")!.click());
  expect(container.querySelector<HTMLButtonElement>("button")!.disabled).toBe(true);
  await setHotkey("Ctrl+Alt+Space");
  invoke.mockImplementation(async (command: string) => {
    if (command === "save_quake_preferences") return { ...desktopDefaults, quakeEnabled: true, quakeHotkey: "CTRL+ALT+SPACE" };
    return { enabled: false, errorCode: null };
  });
  await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
  expect(invoke).toHaveBeenCalledWith("save_quake_preferences", { enabled: true, hotkey: "Ctrl+Alt+Space", heightPercent: 50 });
  expect(useDesktopPreferences.getState().preferences.quakeHotkey).toBe("CTRL+ALT+SPACE");
});

it("shows registration failure while retaining the previously saved shortcut", async () => {
  const previous = { ...desktopDefaults, quakeEnabled: true, quakeHotkey: "CTRL+ALT+F12" };
  useDesktopPreferences.setState({ preferences: previous });
  await act(async () => root.render(<QuakeSettings locale="en" />));
  await setHotkey("Ctrl+Alt+F11");
  invoke.mockRejectedValue({ code: "quake_hotkey_busy" });
  await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
  expect(container.querySelector("[role='alert']")?.textContent).toBe(copy.en.quakeRegistrationError);
  expect(useDesktopPreferences.getState().preferences).toEqual(previous);
});

it("reports startup registration failure and shows Quake as disabled", async () => {
  useDesktopPreferences.setState({ preferences: { ...desktopDefaults, quakeEnabled: true, quakeHotkey: "CTRL+ALT+F12" } });
  invoke.mockResolvedValue({ enabled: false, errorCode: "quake_hotkey_busy" });
  await act(async () => root.render(<QuakeSettings locale="zh-CN" />));
  expect(container.querySelector<HTMLInputElement>("input[type='checkbox']")!.checked).toBe(false);
  expect(container.querySelector("[role='alert']")?.textContent).toBe(copy["zh-CN"].quakeRegistrationError);
});
