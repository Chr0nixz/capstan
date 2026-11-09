export function isMacOS(): boolean {
  return navigator.platform.startsWith("Mac");
}

export function primaryModifier(event: { ctrlKey: boolean; metaKey: boolean }, mac = isMacOS()): boolean {
  return mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}

/** Remote editing and Finder drag-out have no macOS backend yet; their commands are not registered. */
export function supportsRemoteEditing(): boolean {
  return !isMacOS();
}

export function supportsNativeExport(): boolean {
  return !isMacOS();
}

function macShortcut(keys: string): string {
  return keys
    .replace(/Ctrl\+Shift\+([TWF])\b/gu, "Meta+$1")
    .replaceAll("Ctrl+Alt+", "Meta+")
    .replace(/Ctrl\+(?!Tab\b|Shift\+Tab\b)/gu, "Meta+");
}

export function shortcutLabel(keys: string): string {
  return isMacOS()
    ? macShortcut(keys).replaceAll("Meta+", "⌘").replaceAll("Alt+", "⌥").replaceAll("Shift+", "⇧").replaceAll("Ctrl+", "⌃")
    : keys;
}

export function ariaShortcut(keys: string): string {
  return (isMacOS() ? macShortcut(keys) : keys).replaceAll("Ctrl+", "Control+");
}

export type WorkspaceShortcut =
  | "new_terminal" | "close_terminal" | "search_hosts" | "command_palette" | "toggle_sidebar"
  | "focus_files" | "focus_transfers" | "settings" | "next_tab" | "previous_tab" | { tab: number };

type KeyState = { key: string; code?: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean };

const SHARED_SHIFTED: Readonly<Record<string, WorkspaceShortcut>> = {
  k: "search_hosts", p: "command_palette", b: "toggle_sidebar", e: "focus_files", j: "focus_transfers",
};

/**
 * Windows Terminal's keys on Windows, where plain Ctrl+W/T/K/P stay with the
 * shell. macOS uses Command instead, which leaves every Control chord to the shell.
 */
export function workspaceShortcut(event: KeyState, mac = isMacOS()): WorkspaceShortcut | null {
  const key = event.key.toLocaleLowerCase();
  if (event.ctrlKey && !event.metaKey && !event.altKey && key === "tab") return event.shiftKey ? "previous_tab" : "next_tab";
  if (!primaryModifier(event, mac)) return null;
  const digit = /^[1-9]$/u.test(event.key) ? { tab: Number(event.key) } : null;
  if (!event.shiftKey && !event.altKey) {
    if (key === ",") return "settings";
    if (mac && key === "t") return "new_terminal";
    if (mac && key === "w") return "close_terminal";
    return mac ? digit : null;
  }
  if (event.altKey) return !mac && !event.shiftKey ? digit : null;
  if (mac && event.code === "BracketLeft") return "previous_tab";
  if (mac && event.code === "BracketRight") return "next_tab";
  if (!mac && key === "t") return "new_terminal";
  if (!mac && key === "w") return "close_terminal";
  return SHARED_SHIFTED[key] ?? null;
}

/** Line-editing chords macOS users expect at a prompt; xterm sends nothing for Command keys. */
export function macTerminalSequence(event: KeyState): string | null {
  if (!event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return null;
  if (event.key === "ArrowLeft") return "\x01";
  if (event.key === "ArrowRight") return "\x05";
  if (event.key === "Backspace") return "\x15";
  return null;
}
