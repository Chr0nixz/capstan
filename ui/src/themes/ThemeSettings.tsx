import { useEffect, useRef, useState, type ReactElement } from "react";
import { copy } from "../i18n";
import { invokeTauri } from "../terminal/bridge";
import { useConfirm } from "../ui/ConfirmDialog";
import { saveDesktopPreference, useDesktopPreferences, useTerminalThemes } from "./store";
import type { DesktopPreference, TerminalThemeDefinition, ThemePreview } from "./contracts";
import { QuakeSettings } from "../window/QuakeSettings";

type Locale = keyof typeof copy;

function errorLabel(cause: unknown, labels: typeof copy[Locale]): string {
  const code = typeof cause === "object" && cause !== null && "code" in cause ? cause.code : null;
  switch (code) {
    case "theme_limit": return labels.themeLimit;
    case "theme_json": return labels.themeJsonInvalid;
    case "theme_xml": return labels.themeXmlInvalid;
    case "theme_invalid": return labels.themeInvalid;
    case "theme_changed": return labels.themeCatalogChanged;
    default: return labels.themeUnavailable;
  }
}

export function ThemeSettings({ locale }: { locale: Locale }): ReactElement {
  const labels = copy[locale];
  const { preferences, ready, saving, loadError: preferencesLoadError } = useDesktopPreferences();
  const { themes, loadError } = useTerminalThemes();
  const [preview, setPreview] = useState<ThemePreview | null>(null);
  const [previewIndex, setPreviewIndex] = useState(0);
  const [editing, setEditing] = useState<TerminalThemeDefinition | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preferenceError, setPreferenceError] = useState(false);
  const [recentCleared, setRecentCleared] = useState(false);
  const previewRef = useRef<ThemePreview | null>(null);
  const mounted = useRef(false);
  const importButton = useRef<HTMLButtonElement>(null);
  const previewHeading = useRef<HTMLHeadingElement>(null);
  const [confirm, confirmDialog] = useConfirm();

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const token = previewRef.current?.token;
      if (token) void invokeTauri("discard_terminal_theme_preview", { token }).catch(() => undefined);
    };
  }, []);
  useEffect(() => { if (preview) previewHeading.current?.focus(); }, [preview]);

  const changePreference = (preference: DesktopPreference, value: boolean): void => {
    setPreferenceError(false);
    void saveDesktopPreference(preference, value).catch(() => setPreferenceError(true));
  };

  const actOnCatalog = async (operation: () => Promise<void>): Promise<void> => {
    setBusy(true);
    setError(null);
    try { await operation(); }
    catch (cause) { if (mounted.current) setError(errorLabel(cause, labels)); }
    finally { if (mounted.current) setBusy(false); }
  };

  const pickTheme = (): void => {
    void actOnCatalog(async () => {
      const selected = await invokeTauri("preview_terminal_themes", { locale });
      if (!selected) return;
      if (!mounted.current) {
        await invokeTauri("discard_terminal_theme_preview", { token: selected.token });
        return;
      }
      previewRef.current = selected;
      setPreviewIndex(0);
      setPreview(selected);
    });
  };

  const closePreview = (): void => {
    previewRef.current = null;
    setPreview(null);
    requestAnimationFrame(() => importButton.current?.focus());
  };

  const applyCatalog = (catalog: TerminalThemeDefinition[]): void => {
    useTerminalThemes.setState({ themes: catalog, loadError: false });
  };

  const shown = preview?.themes[previewIndex];

  return <section className="settings-section">
    <h3>{labels.settingsDesktop}</h3>
    <div className="settings-group">
      <label className="check-row"><input type="checkbox" checked={preferences.copyOnSelect} disabled={!ready || saving}
        onChange={(event) => changePreference("copyOnSelect", event.currentTarget.checked)} />{labels.copyOnSelect}</label>
      <label className="check-row"><input type="checkbox" checked={preferences.systemNotifications} disabled={!ready || saving}
        onChange={(event) => changePreference("systemNotifications", event.currentTarget.checked)} />{labels.systemNotifications}</label>
      <label className="check-row"><input type="checkbox" checked={preferences.jumpListRecentHosts} disabled={!ready || saving}
        onChange={(event) => changePreference("jumpListRecentHosts", event.currentTarget.checked)} />{labels.jumpListRecentHosts}</label>
      <p className="quiet-copy">{labels.jumpListPrivacy}</p>
      <button type="button" disabled={!ready || saving || busy} onClick={() => {
        setBusy(true); setPreferenceError(false); setRecentCleared(false);
        void invokeTauri("clear_recent_hosts").then(() => setRecentCleared(true))
          .catch(() => setPreferenceError(true)).finally(() => setBusy(false));
      }}>{labels.clearRecentHosts}</button>
      {recentCleared && <p role="status" className="quiet-copy">{labels.recentHostsCleared}</p>}
      {(preferenceError || preferencesLoadError) && <p className="sftp-dialog-error" role="alert">{labels.desktopPreferencesError}</p>}
    </div>
    <QuakeSettings locale={locale} />
    <div className="settings-group">
      <h4>{labels.terminalTheme}</h4>
      <p className="quiet-copy">{labels.themeFormats}</p>
      <div className="settings-actions"><button ref={importButton} type="button" disabled={busy || !!preview || !!editing} onClick={pickTheme}>{labels.importTheme}</button></div>
      {busy && <p role="status" className="quiet-copy">{labels.themeWorking}</p>}
      {(error || loadError) && <p className="sftp-dialog-error" role="alert">{error ?? labels.themeUnavailable}</p>}
      {preview && shown && <section className="theme-preview" aria-labelledby="theme-preview-heading">
        <h5 ref={previewHeading} id="theme-preview-heading" tabIndex={-1}>{labels.themePreview}</h5>
        {preview.themes.length > 1
          ? <label>{labels.terminalTheme}<select value={previewIndex} onChange={(event) => setPreviewIndex(Number(event.currentTarget.value))}>
            {preview.themes.map((theme, index) => <option key={theme.id} value={index}>{theme.name}</option>)}
          </select></label>
          : <strong className="theme-name">{shown.name}</strong>}
        <div className="theme-sample" style={{ color: shown.foreground, backgroundColor: shown.background }}>
          <code>{labels.themeSampleCommand}</code><span className="theme-sample-selection" style={{ backgroundColor: shown.selectionBackground }}>{labels.themeSelectedText}</span>
          <span className="theme-sample-cursor" aria-hidden="true" style={{ backgroundColor: shown.cursor }} />
        </div>
        <div className="theme-swatches" role="img" aria-label={labels.themeAnsiColors}>
          {shown.ansi.map((color, index) => <span key={index} style={{ backgroundColor: color }} title={color} />)}
        </div>
        <div className="settings-actions">
          <button type="button" className="ghost-button" disabled={busy} onClick={() => {
            void actOnCatalog(async () => { await invokeTauri("discard_terminal_theme_preview", { token: preview.token }); closePreview(); });
          }}>{labels.cancelEdit}</button>
          <button type="button" className="primary-action" disabled={busy} onClick={() => {
            void actOnCatalog(async () => { applyCatalog(await invokeTauri("save_terminal_theme_preview", { token: preview.token })); closePreview(); });
          }}>{labels.saveImportedThemes(preview.themes.length)}</button>
        </div>
      </section>}
      {themes.length === 0 && !preview && <p className="quiet-copy">{labels.noImportedThemes}</p>}
      {themes.length > 0 && <ul className="item-list theme-list" aria-label={labels.importedThemes}>
        {themes.map((theme) => <li key={theme.id}>
          <div className="theme-list-row">
            <span className="theme-name" title={theme.name}>{theme.name}</span>
            <button type="button" className="ghost-button" disabled={busy || !!preview || !!editing} onClick={() => { setEditing(theme); setName(theme.name); }}>{labels.renameTheme}</button>
            <button type="button" className="ghost-button" disabled={busy || !!preview || !!editing} onClick={() => {
              void (async () => {
                if (await confirm({ title: labels.deleteThemeTitle(theme.name), body: labels.deleteThemeBody, confirm: labels.deleteTheme, cancel: labels.cancelEdit })) {
                  await actOnCatalog(async () => {
                    applyCatalog(await invokeTauri("delete_terminal_theme", { id: theme.id }));
                    requestAnimationFrame(() => importButton.current?.focus());
                  });
                }
              })();
            }}>{labels.deleteTheme}</button>
          </div>
          {editing?.id === theme.id && <form className="theme-rename" onSubmit={(event) => {
            event.preventDefault();
            void actOnCatalog(async () => {
              applyCatalog(await invokeTauri("rename_terminal_theme", { id: theme.id, name }));
              setEditing(null);
              requestAnimationFrame(() => importButton.current?.focus());
            });
          }}>
            <label>{labels.themeName}<input autoFocus value={name} onChange={(event) => setName(event.currentTarget.value)} required /></label>
            <div className="settings-actions">
              <button type="button" className="ghost-button" disabled={busy} onClick={() => { setEditing(null); requestAnimationFrame(() => importButton.current?.focus()); }}>{labels.cancelEdit}</button>
              <button type="submit" disabled={busy || !name.trim() || [...name].length > 64}>{labels.renameTheme}</button>
            </div>
          </form>}
        </li>)}
      </ul>}
    </div>
    {confirmDialog}
  </section>;
}
