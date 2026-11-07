import { useEffect, useState, type ReactElement } from "react";
import { copy } from "../i18n";
import { invokeTauri } from "../terminal/bridge";
import { useDesktopPreferences } from "../themes/store";

type Locale = keyof typeof copy;

export function QuakeSettings({ locale }: { locale: Locale }): ReactElement {
  const labels = copy[locale];
  const { preferences, ready, saving } = useDesktopPreferences();
  const [enabled, setEnabled] = useState(preferences.quakeEnabled);
  const [hotkey, setHotkey] = useState(preferences.quakeHotkey ?? "");
  const [height, setHeight] = useState(preferences.quakeHeightPercent);
  const [savingLocal, setSavingLocal] = useState(false);
  const [error, setError] = useState<"busy" | "settings" | null>(null);

  useEffect(() => {
    setEnabled(preferences.quakeEnabled);
    setHotkey(preferences.quakeHotkey ?? "");
    setHeight(preferences.quakeHeightPercent);
  }, [preferences.quakeEnabled, preferences.quakeHotkey, preferences.quakeHeightPercent]);

  useEffect(() => {
    let active = true;
    void invokeTauri("quake_status").then((status) => {
      if (active && status.errorCode === "quake_hotkey_busy") {
        setError("busy");
        if (!status.enabled) useDesktopPreferences.setState((current) => ({ preferences: { ...current.preferences, quakeEnabled: false } }));
      }
    }).catch(() => undefined);
    return () => { active = false; };
  }, []);

  const save = async (): Promise<void> => {
    if (savingLocal || (enabled && hotkey.trim() === "")) return;
    setSavingLocal(true);
    useDesktopPreferences.setState({ saving: true });
    setError(null);
    try {
      const next = await invokeTauri("save_quake_preferences", {
        enabled, hotkey: hotkey.trim() || null, heightPercent: height,
      });
      useDesktopPreferences.setState({ preferences: next });
    } catch (cause) {
      const code = typeof cause === "object" && cause !== null && "code" in cause ? cause.code : null;
      setError(code === "quake_hotkey_busy" ? "busy" : "settings");
    } finally {
      setSavingLocal(false);
      useDesktopPreferences.setState({ saving: false });
    }
  };

  return <section className="settings-section quake-settings">
    <h4>{labels.quakeSettings}</h4>
    <div className="settings-group">
      <label className="check-row"><input type="checkbox" checked={enabled} disabled={!ready || saving || savingLocal}
        onChange={(event) => { setEnabled(event.currentTarget.checked); setError(null); }} />{labels.quakeEnabled}</label>
      <label>{labels.quakeHotkey}<input className="mono" value={hotkey} onChange={(event) => {
        setHotkey(event.currentTarget.value); setError(null);
      }} maxLength={64} placeholder={labels.quakeHotkeyPlaceholder} spellCheck={false} disabled={!ready || saving || savingLocal} /></label>
      <p className="quiet-copy">{labels.quakeHotkeyHint}</p>
      <label className="quake-height-label">{labels.quakeHeight}<span className="mono">{labels.quakeHeightValue(height)}</span></label>
      <input aria-label={labels.quakeHeight} type="range" min={20} max={90} step={1} value={height}
        onChange={(event) => setHeight(Number(event.currentTarget.value))} disabled={!ready || saving || savingLocal} />
      {error === "busy" && <p className="sftp-dialog-error" role="alert">{labels.quakeRegistrationError}</p>}
      {error === "settings" && <p className="sftp-dialog-error" role="alert">{labels.quakeSettingsError}</p>}
      <div className="settings-actions"><button className="primary-action" type="button" onClick={() => { void save(); }}
        disabled={!ready || saving || savingLocal || (enabled && hotkey.trim() === "")}>{labels.quakeSave}</button></div>
    </div>
  </section>;
}
