import { useState, type ReactElement } from "react";
import { copy } from "../i18n";
import { invokeTauri } from "../terminal/bridge";

type Locale = keyof typeof copy;

/** The log's location stays in Rust; the button asks Rust to show it. */
export function DiagnosticsSettings({ locale }: { locale: Locale }): ReactElement {
  const labels = copy[locale];
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  return <section className="settings-section" aria-labelledby="diagnostics-settings-title">
    <h3 id="diagnostics-settings-title">{labels.diagnostics}</h3>
    <div className="settings-group">
      <p className="quiet-copy">{labels.diagnosticsHint}</p>
      <div className="settings-actions">
        <button type="button" disabled={busy} onClick={() => {
          setBusy(true); setError(false);
          void invokeTauri("open_diagnostics_folder").catch(() => setError(true)).finally(() => setBusy(false));
        }}>{labels.openDiagnostics}</button>
      </div>
      {error && <p className="sftp-dialog-error" role="alert">{labels.diagnosticsError}</p>}
    </div>
  </section>;
}
