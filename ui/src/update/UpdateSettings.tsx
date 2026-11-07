import { useEffect, useRef, type ReactElement } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { copy } from "../i18n";
import { invokeTauri, isTauriAvailable } from "../terminal/bridge";
import { useConfirm } from "../ui/ConfirmDialog";
import { checkUpdates, refreshUpdates, useUpdates } from "./store";

type Locale = keyof typeof copy;
function errorMessage(code: string | null, labels: typeof copy[Locale]["update"]): string {
  switch (code) {
    case "update_manifest": return labels.manifest;
    case "update_version": return labels.version;
    case "update_limit": return labels.limit;
    case "update_signature": return labels.signature;
    case "update_storage": return labels.storage;
    case "update_canceled": return labels.canceled;
    case "update_install": return labels.installError;
    case "update_busy": return labels.busy;
    case "update_confirmation": return labels.confirmActivity;
    default: return labels.network;
  }
}

export function UpdateSettings({ locale }: { locale: Locale }): ReactElement {
  const labels = copy[locale].update;
  const { status, saving, preferenceError, loadError } = useUpdates();
  const busy = !!status && ["checking", "downloading", "installing"].includes(status.phase);
  const savePreference = async (enabled: boolean): Promise<void> => {
    useUpdates.setState({ saving: true, preferenceError: false });
    try {
      await invokeTauri("save_update_preference", { enabled });
      await refreshUpdates();
    } catch { useUpdates.setState({ preferenceError: true }); }
    finally { useUpdates.setState({ saving: false }); }
  };
  return <section className="settings-section update-settings" aria-labelledby="update-settings-title">
    <h3 id="update-settings-title">{labels.title}</h3>
    <div className="settings-group">
      <p className="quiet-copy">{status?.available ? status.portable ? labels.portable : labels.manual : labels.unavailable}</p>
      <label className="check-row"><input type="checkbox" checked={status?.checkOnStartup ?? false}
        disabled={!status?.available || saving || busy} onChange={(event) => { void savePreference(event.currentTarget.checked); }} />{labels.startup}</label>
      <div className="settings-actions">
        <button type="button" disabled={!status?.available || busy} onClick={() => { void checkUpdates(); }}>{labels.check}</button>
        {status?.release && <button type="button" onClick={() => useUpdates.setState({ dialogOpen: true })}>{labels.details}</button>}
      </div>
      <p role="status" className="quiet-copy">{status?.phase === "checking" ? labels.checking : status?.phase === "idle" && status.available ? labels.ready : ""}</p>
      {status?.errorCode && <p role="alert" className="sftp-dialog-error">{errorMessage(status.errorCode, labels)}</p>}
      {(loadError || preferenceError) && <p role="alert" className="sftp-dialog-error">{labels.preferencesError}</p>}
    </div>
  </section>;
}

/** Mounted once: startup results remain visible even while the Vault is locked. */
export function UpdateCenter({ locale }: { locale: Locale }): ReactElement | null {
  const labels = copy[locale];
  const { status, dialogOpen } = useUpdates();
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [confirm, confirmDialog] = useConfirm();
  const busy = !!status && ["checking", "downloading", "installing"].includes(status.phase);
  useEffect(() => { if (isTauriAvailable()) void refreshUpdates(); }, []);
  useEffect(() => {
    if (!busy) return;
    const timer = window.setInterval(() => { void refreshUpdates(); }, 500);
    return () => window.clearInterval(timer);
  }, [busy]);

  const install = async (): Promise<void> => {
    if (!status?.release || status.portable || busy) return;
    const activity = await invokeTauri("update_activity").catch(() => true);
    if (activity && !await confirm({ title: labels.update.title, body: labels.update.confirmActivity,
      confirm: labels.update.install, cancel: labels.cancelEdit, tone: "primary" })) return;
    useUpdates.setState({ status: { ...status, phase: "downloading", errorCode: null } });
    try { await invokeTauri("install_update", { id: status.release.id, confirmed: true, confirmActivity: activity }); }
    catch { await refreshUpdates(); }
  };
  if (!status?.release) return null;
  return <>
    <div className="update-notice" role="status"><span>{labels.update.found(status.release.version)}</span>
      <button type="button" onClick={() => useUpdates.setState({ dialogOpen: true })}>{labels.update.details}</button></div>
    <Dialog.Root open={dialogOpen} onOpenChange={(open) => { if (status.phase !== "installing") useUpdates.setState({ dialogOpen: open }); }}>
      <Dialog.Portal><Dialog.Overlay className="sftp-dialog-overlay" />
        <Dialog.Content className="sftp-dialog update-dialog" onOpenAutoFocus={(event) => { event.preventDefault(); cancelRef.current?.focus(); }}
          onEscapeKeyDown={(event) => { if (status.phase === "installing") event.preventDefault(); }}>
          <Dialog.Title>{labels.update.found(status.release.version)}</Dialog.Title>
          <Dialog.Description>{status.portable ? labels.update.portable : labels.update.confirm}</Dialog.Description>
          <pre className="update-notes">{status.release.notes || labels.update.noNotes}</pre>
          {status.phase === "downloading" && <label className="update-progress">{labels.update.downloading}
            <progress max={status.total || 1} value={status.downloaded} />
            <span className="mono">{Math.floor(status.downloaded / Math.max(status.total, 1) * 100)}%</span></label>}
          {status.phase === "installing" && <p role="status">{labels.update.installing}</p>}
          {status.errorCode && <p role="alert" className="sftp-dialog-error">{errorMessage(status.errorCode, labels.update)}</p>}
          <div className="sftp-dialog-actions">
            {status.phase === "downloading" ? <button ref={cancelRef} type="button" onClick={() => { void invokeTauri("cancel_update").catch(() => undefined); }}>{labels.update.cancelDownload}</button>
              : <button ref={cancelRef} type="button" disabled={status.phase === "installing"} onClick={() => useUpdates.setState({ dialogOpen: false })}>{labels.cancelEdit}</button>}
            {!status.portable && <button className="primary-action" type="button" disabled={busy} onClick={() => { void install(); }}>{labels.update.install}</button>}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
    {confirmDialog}
  </>;
}
