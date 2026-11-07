export type TerminalTheme = "graphite" | "light" | `custom:${string}`;
export type CustomThemeId = `custom:${string}`;
export type ThemeSource = "windows_terminal" | "iterm2";

export type TerminalThemeDefinition = {
  id: CustomThemeId;
  name: string;
  source: ThemeSource;
  foreground: string;
  background: string;
  cursor: string;
  selectionBackground: string;
  ansi: readonly string[];
};

export type ThemePreview = { token: string; themes: TerminalThemeDefinition[] };
export type DesktopPreferences = {
  copyOnSelect: boolean;
  systemNotifications: boolean;
  jumpListRecentHosts: boolean;
  quakeEnabled: boolean;
  quakeHotkey: string | null;
  quakeHeightPercent: number;
};
export type DesktopPreference = "copyOnSelect" | "systemNotifications" | "jumpListRecentHosts";

const customId = /^custom:[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/u;

export function isTerminalTheme(value: unknown): value is TerminalTheme {
  return value === "graphite" || value === "light" || (
    typeof value === "string" && customId.test(value) && value !== "custom:00000000-0000-0000-0000-000000000000"
  );
}

export function terminalThemeId(value: string): TerminalTheme {
  return isTerminalTheme(value) ? value : "graphite";
}
