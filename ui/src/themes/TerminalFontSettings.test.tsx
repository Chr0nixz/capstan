import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TerminalFontSettings } from "./TerminalFontSettings";
import { desktopDefaults, useDesktopPreferences } from "./store";
import { copy } from "../i18n";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../terminal/bridge", () => ({ invokeTauri: invoke }));
Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
let root: Root;
let container: HTMLDivElement;
beforeEach(() => {
  invoke.mockReset();
  useDesktopPreferences.setState({ preferences: desktopDefaults, ready: true, saving: false });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

async function type(input: HTMLInputElement, value: string): Promise<void> {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
const family = (): HTMLInputElement => container.querySelector<HTMLInputElement>("input.mono")!;
const size = (): HTMLInputElement => container.querySelector<HTMLInputElement>("input[type='number']")!;
const button = (label: string): HTMLButtonElement => [...container.querySelectorAll("button")].find((item) => item.textContent === label)!;

it("saves a trimmed family and size, and resets both to the defaults", async () => {
  await act(async () => root.render(<TerminalFontSettings locale="en" />));
  expect(family().value).toBe("");
  expect(size().value).toBe("15");
  invoke.mockImplementation(async (_command: string, args: { family: string | null; size: number }) =>
    ({ ...desktopDefaults, terminalFontFamily: args.family, terminalFontSize: args.size }));
  await type(family(), '  "JetBrains Mono", monospace ');
  await type(size(), "13");
  await act(async () => button(copy.en.terminalFontApply).click());
  expect(invoke).toHaveBeenLastCalledWith("save_terminal_font", { family: '"JetBrains Mono", monospace', size: 13 });
  expect(useDesktopPreferences.getState().preferences.terminalFontSize).toBe(13);
  expect(family().value).toBe('"JetBrains Mono", monospace');

  await act(async () => button(copy.en.terminalFontReset).click());
  expect(invoke).toHaveBeenLastCalledWith("save_terminal_font", { family: null, size: 15 });
  expect(family().value).toBe("");
});

it("refuses an out-of-range size or a family that could escape the font string without saving", async () => {
  await act(async () => root.render(<TerminalFontSettings locale="en" />));
  for (const [nextFamily, nextSize] of [["Menlo", "40"], ["Menlo", "12.5"], ["Menlo; color: red", "13"], ["a\\b", "13"]] as const) {
    await type(family(), nextFamily);
    await type(size(), nextSize);
    await act(async () => button(copy.en.terminalFontApply).click());
    expect(container.querySelector("[role='alert']")?.textContent).toBe(copy.en.terminalFontError);
  }
  expect(invoke).not.toHaveBeenCalled();
});

it("shows the save error when Rust rejects the font", async () => {
  await act(async () => root.render(<TerminalFontSettings locale="zh-CN" />));
  invoke.mockRejectedValue({ code: "settings_invalid" });
  await type(family(), "Menlo");
  await act(async () => button(copy["zh-CN"].terminalFontApply).click());
  expect(container.querySelector("[role='alert']")?.textContent).toBe(copy["zh-CN"].terminalFontError);
  expect(useDesktopPreferences.getState().saving).toBe(false);
});
