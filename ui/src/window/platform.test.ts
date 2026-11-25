import { afterEach, describe, expect, it, vi } from "vitest";
import { ariaShortcut, isMacOS, macTerminalSequence, primaryModifier, shortcutLabel, workspaceShortcut } from "./platform";

const key = (value: string, modifiers: Partial<Record<"ctrlKey" | "metaKey" | "altKey" | "shiftKey", boolean>> = {}, code?: string) => ({
  key: value, code, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...modifiers,
});
const onMac = () => vi.spyOn(navigator, "platform", "get").mockReturnValue("MacIntel");

afterEach(() => { vi.restoreAllMocks(); });

describe("platform detection", () => {
  it("recognizes macOS only from the Mac platform", () => {
    vi.spyOn(navigator, "platform", "get").mockReturnValue("Win32");
    expect(isMacOS()).toBe(false);
    onMac();
    expect(isMacOS()).toBe(true);
  });

  it("uses Command as the primary modifier only on macOS", () => {
    expect(primaryModifier(key("a", { metaKey: true }), true)).toBe(true);
    expect(primaryModifier(key("a", { ctrlKey: true }), true)).toBe(false);
    expect(primaryModifier(key("a", { ctrlKey: true, metaKey: true }), true)).toBe(false);
    expect(primaryModifier(key("a", { ctrlKey: true }), false)).toBe(true);
    expect(primaryModifier(key("a", { metaKey: true }), false)).toBe(false);
  });
});

describe("shortcut labels", () => {
  it("leaves every label alone off macOS", () => {
    vi.spyOn(navigator, "platform", "get").mockReturnValue("Win32");
    for (const keys of ["Ctrl+Shift+T", "Ctrl+Alt+1–8", "Alt+Shift+=", "F6 / Shift+F6"]) expect(shortcutLabel(keys)).toBe(keys);
    expect(ariaShortcut("Ctrl+Shift+P")).toBe("Control+Shift+P");
  });

  it("shows the keys the macOS handler actually accepts", () => {
    onMac();
    expect(shortcutLabel("Ctrl+Shift+T")).toBe("⌘T");
    expect(shortcutLabel("Ctrl+Shift+W")).toBe("⌘W");
    expect(shortcutLabel("Ctrl+Shift+F")).toBe("⌘F");
    expect(shortcutLabel("Ctrl+Shift+P")).toBe("⌘⇧P");
    expect(shortcutLabel("Ctrl+,")).toBe("⌘,");
    expect(shortcutLabel("Ctrl+Alt+1–8, Ctrl+Alt+9")).toBe("⌘1–8, ⌘9");
    expect(shortcutLabel("Ctrl+Alt+Shift+T")).toBe("⌘⇧T");
    expect(shortcutLabel("Alt+Shift+=")).toBe("⌥⇧=");
    expect(shortcutLabel("Ctrl+click")).toBe("⌘click");
    expect(shortcutLabel("Ctrl+Tab / Ctrl+Shift+Tab")).toBe("⌃Tab / ⌃⇧Tab");
    expect(ariaShortcut("Ctrl+Shift+W")).toBe("Meta+W");
    expect(ariaShortcut("Ctrl+Comma")).toBe("Meta+Comma");
  });
});

describe("workspace shortcuts", () => {
  it("keeps the Windows Terminal keys on Windows", () => {
    const windows = (value: string, modifiers: Parameters<typeof key>[1]) => workspaceShortcut(key(value, modifiers), false);
    expect(windows("T", { ctrlKey: true, shiftKey: true })).toBe("new_terminal");
    expect(windows("W", { ctrlKey: true, shiftKey: true })).toBe("close_terminal");
    expect(windows("K", { ctrlKey: true, shiftKey: true })).toBe("search_hosts");
    expect(windows("P", { ctrlKey: true, shiftKey: true })).toBe("command_palette");
    expect(windows("B", { ctrlKey: true, shiftKey: true })).toBe("toggle_sidebar");
    expect(windows("E", { ctrlKey: true, shiftKey: true })).toBe("focus_files");
    expect(windows("J", { ctrlKey: true, shiftKey: true })).toBe("focus_transfers");
    expect(windows(",", { ctrlKey: true })).toBe("settings");
    expect(windows("Tab", { ctrlKey: true })).toBe("next_tab");
    expect(windows("Tab", { ctrlKey: true, shiftKey: true })).toBe("previous_tab");
    expect(windows("3", { ctrlKey: true, altKey: true })).toEqual({ tab: 3 });
    expect(windows("T", { ctrlKey: true, altKey: true, shiftKey: true })).toBe("reopen_terminal");
  });

  it("leaves plain Control chords to the shell on Windows", () => {
    for (const value of ["t", "w", "k", "p", "3"]) expect(workspaceShortcut(key(value, { ctrlKey: true }), false)).toBeNull();
    expect(workspaceShortcut(key("t", { metaKey: true }), false)).toBeNull();
    expect(workspaceShortcut(key("T", { ctrlKey: true, metaKey: true, shiftKey: true }), false)).toBeNull();
  });

  it("maps the same actions to Command on macOS", () => {
    const mac = (value: string, modifiers: Parameters<typeof key>[1], code?: string) => workspaceShortcut(key(value, modifiers, code), true);
    expect(mac("t", { metaKey: true })).toBe("new_terminal");
    expect(mac("w", { metaKey: true })).toBe("close_terminal");
    expect(mac("T", { metaKey: true, shiftKey: true })).toBe("reopen_terminal");
    expect(mac("T", { metaKey: true, altKey: true, shiftKey: true })).toBeNull();
    expect(mac("3", { metaKey: true })).toEqual({ tab: 3 });
    expect(mac(",", { metaKey: true })).toBe("settings");
    expect(mac("P", { metaKey: true, shiftKey: true })).toBe("command_palette");
    expect(mac("K", { metaKey: true, shiftKey: true })).toBe("search_hosts");
    expect(mac("{", { metaKey: true, shiftKey: true }, "BracketLeft")).toBe("previous_tab");
    expect(mac("}", { metaKey: true, shiftKey: true }, "BracketRight")).toBe("next_tab");
    expect(mac("Tab", { ctrlKey: true })).toBe("next_tab");
  });

  it("never takes Control chords or system Command keys on macOS", () => {
    for (const value of ["t", "w", "k", "p", "c", "d", "r", "z", "3"]) expect(workspaceShortcut(key(value, { ctrlKey: true }), true)).toBeNull();
    for (const value of ["q", "h", "m", "c", "v", "x", "a", "z"]) expect(workspaceShortcut(key(value, { metaKey: true }), true)).toBeNull();
    expect(workspaceShortcut(key("T", { ctrlKey: true, shiftKey: true }), true)).toBeNull();
    expect(workspaceShortcut(key("3", { metaKey: true, altKey: true }), true)).toBeNull();
  });
});

describe("macOS terminal chords", () => {
  it("sends line-editing bytes only for an unmodified Command chord", () => {
    expect(macTerminalSequence(key("ArrowLeft", { metaKey: true }))).toBe("\x01");
    expect(macTerminalSequence(key("ArrowRight", { metaKey: true }))).toBe("\x05");
    expect(macTerminalSequence(key("Backspace", { metaKey: true }))).toBe("\x15");
    expect(macTerminalSequence(key("ArrowLeft"))).toBeNull();
    expect(macTerminalSequence(key("ArrowLeft", { metaKey: true, shiftKey: true }))).toBeNull();
    expect(macTerminalSequence(key("c", { metaKey: true }))).toBeNull();
  });
});
