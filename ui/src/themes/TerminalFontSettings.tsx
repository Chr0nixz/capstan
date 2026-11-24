import { useEffect, useState, type ReactElement } from "react";
import { copy } from "../i18n";
import { DEFAULT_FONT_SIZE, defaultFontFamily } from "../terminal/session";
import { saveTerminalFont, useDesktopPreferences } from "./store";

type Locale = keyof typeof copy;

const MIN_SIZE = 8;
const MAX_SIZE = 32;
// Rust refuses these too; they could end or extend xterm's canvas font string.
const UNSAFE_FAMILY = /[;{}<>\\\u0000-\u001f\u007f]/u;

export function TerminalFontSettings({ locale }: { locale: Locale }): ReactElement {
  const labels = copy[locale];
  const { preferences, ready, saving } = useDesktopPreferences();
  const [family, setFamily] = useState(preferences.terminalFontFamily ?? "");
  const [size, setSize] = useState(String(preferences.terminalFontSize));
  const [error, setError] = useState(false);

  useEffect(() => {
    setFamily(preferences.terminalFontFamily ?? "");
    setSize(String(preferences.terminalFontSize));
  }, [preferences.terminalFontFamily, preferences.terminalFontSize]);

  const parsedSize = Number(size);
  const valid = Number.isInteger(parsedSize) && parsedSize >= MIN_SIZE && parsedSize <= MAX_SIZE && !UNSAFE_FAMILY.test(family);
  const disabled = !ready || saving;

  const save = (nextFamily: string | null, nextSize: number): void => {
    setError(false);
    void saveTerminalFont(nextFamily, nextSize).catch(() => setError(true));
  };

  return <form className="settings-group terminal-font-settings" aria-label={labels.terminalFont} noValidate onSubmit={(event) => {
    event.preventDefault();
    if (valid) save(family.trim() || null, parsedSize);
    else setError(true);
  }}>
    <h4>{labels.terminalFont}</h4>
    <label>{labels.terminalFontFamily}<input className="mono" value={family} placeholder={defaultFontFamily()} maxLength={256}
      spellCheck={false} disabled={disabled} onChange={(event) => { setFamily(event.currentTarget.value); setError(false); }} /></label>
    <p className="quiet-copy">{labels.terminalFontFamilyHint}</p>
    <label>{labels.terminalFontSize}<input type="number" inputMode="numeric" min={MIN_SIZE} max={MAX_SIZE} step={1} value={size}
      disabled={disabled} onChange={(event) => { setSize(event.currentTarget.value); setError(false); }} /></label>
    {error && <p className="sftp-dialog-error" role="alert">{labels.terminalFontError}</p>}
    <div className="settings-actions">
      <button type="button" className="ghost-button" disabled={disabled} onClick={() => save(null, DEFAULT_FONT_SIZE)}>{labels.terminalFontReset}</button>
      <button type="submit" className="primary-action" disabled={disabled}>{labels.terminalFontApply}</button>
    </div>
  </form>;
}
