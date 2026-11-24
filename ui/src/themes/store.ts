import { create } from "zustand";
import { invokeTauri } from "../terminal/bridge";
import type { DesktopPreference, DesktopPreferences, TerminalThemeDefinition } from "./contracts";

export const desktopDefaults: DesktopPreferences = {
  copyOnSelect: false, systemNotifications: true, jumpListRecentHosts: false,
  quakeEnabled: false, quakeHotkey: null, quakeHeightPercent: 50,
  terminalFontFamily: null, terminalFontSize: 15,
};

export const useDesktopPreferences = create<{
  preferences: DesktopPreferences;
  ready: boolean;
  saving: boolean;
  loadError: boolean;
}>(() => ({ preferences: desktopDefaults, ready: false, saving: false, loadError: false }));

export const useTerminalThemes = create<{
  themes: TerminalThemeDefinition[];
  loadError: boolean;
}>(() => ({ themes: [], loadError: false }));

export async function saveDesktopPreference(preference: DesktopPreference, value: boolean): Promise<void> {
  if (useDesktopPreferences.getState().saving) return;
  useDesktopPreferences.setState({ saving: true });
  try {
    const preferences = await invokeTauri("save_desktop_preference", { preference, value });
    useDesktopPreferences.setState({ preferences });
  } finally {
    useDesktopPreferences.setState({ saving: false });
  }
}

export async function saveTerminalFont(family: string | null, size: number): Promise<void> {
  if (useDesktopPreferences.getState().saving) return;
  useDesktopPreferences.setState({ saving: true });
  try {
    const preferences = await invokeTauri("save_terminal_font", { family, size });
    useDesktopPreferences.setState({ preferences });
  } finally {
    useDesktopPreferences.setState({ saving: false });
  }
}
