import { useEffect, useState, useSyncExternalStore, type ReactElement } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { isTauriAvailable } from "../terminal/bridge";
import type { TerminalTheme } from "../terminal/contracts";
import { sftpCopy } from "../sftp/copy";
import { remoteEditors } from "./controller";
import { editorCopy } from "./copy";
import { transferCopy } from "../transfers/copy";
import type { RetryBinding } from "./transport";
import { ChevronRightIcon, ExternalEditIcon } from "../ui/icons";

export function RemoteEditors({ locale, hosts, theme, sessions: bindings = [], onRetrySessionRequested }: { locale: "en" | "zh-CN"; hosts: Readonly<Record<string, string>>; theme: TerminalTheme; sessions?: readonly RetryBinding[]; onRetrySessionRequested?: (hostId: string) => void }): ReactElement | null {
  const state = useSyncExternalStore(remoteEditors.store.subscribe, remoteEditors.store.getState, remoteEditors.store.getState);
  const [dialog, setDialog] = useState<{ id: string; conflict: boolean } | null>(null);
  const [expanded, setExpanded] = useState(() => window.innerHeight >= 480);
  const [name, setName] = useState("");
  const [requested, setRequested] = useState<{ id: string; cleanup: boolean } | null>(null);
  const labels = editorCopy[locale], errors = sftpCopy[locale].errors;
  const selected = state.snapshot?.sessions.find((session) => session.id === dialog?.id);
  const busy = !!selected && state.pending.includes(selected.id);
  const safely = async (action: () => Promise<unknown>): Promise<void> => { try { await action(); } catch { /* Controller retains the stable error. */ } };
  useEffect(() => {
    if (!isTauriAvailable()) return;
    remoteEditors.start();
    const timer = window.setInterval(() => void remoteEditors.refresh(), 500);
    return () => { window.clearInterval(timer); remoteEditors.dispose(); };
  }, []);
  useEffect(() => {
    if (!requested) return;
    const edit = state.snapshot?.sessions.find((edit) => edit.id === requested.id);
    if (!edit) { setRequested(null); return; }
    const binding = bindings.find((binding) => binding.hostId === edit?.hostId);
    if (!binding) return;
    setRequested(null);
    void safely(() => requested.cleanup ? remoteEditors.cleanup(requested.id, binding) : remoteEditors.retry(requested.id, binding));
  }, [requested, bindings, state.snapshot]);
  if (!isTauriAvailable()) return null;
  const sessions = state.snapshot?.sessions ?? [];
  if (sessions.length === 0 && !state.error && !state.snapshot?.recoveryIncomplete) return null;
  return <>
    <section className="transfer-queue remote-edit-queue" data-theme={theme} aria-label={labels.title}>
      <div className="remote-edit-heading">
        <button className="transfer-queue-heading" type="button" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}><ChevronRightIcon size={14} /><span>{labels.title}</span><span className="sr-only"> · </span><span className="count-badge">{sessions.length}</span></button>
        <button type="button" disabled={state.pending.includes("configuration")} onClick={() => void safely(() => remoteEditors.configure(locale))}>{labels.configure}</button>
        {(state.snapshot?.recoveryIncomplete || sessions.some((session) => session.recovered)) && <button type="button" disabled={state.pending.includes("discovery")} onClick={() => void safely(() => remoteEditors.discover())}>{labels.discover}</button>}
      </div>
      {state.error && <p className="sftp-error" role="alert">{errors[state.error]}</p>}
      {requested && <p className="sftp-status" role="status">{requested.cleanup ? labels.reconnectCleanup : labels.reconnect}<button type="button" onClick={() => setRequested(null)}>{sftpCopy[locale].cancel}</button></p>}
      {state.snapshot?.recoveryIncomplete && <p className="sftp-error" role="alert">{labels.recoveryIncomplete}</p>}
      {expanded && <div className="transfer-rows">{sessions.map((session) => <article className="transfer-row" data-state={session.error ? "failed" : session.state} key={session.id}>
        <span className="transfer-direction" aria-hidden="true"><ExternalEditIcon size={14} /></span>
        <div className="transfer-row-context"><strong>{session.name}</strong><span>{session.recovered ? labels.recovered : (session.hostId && hosts[session.hostId]) || transferCopy[locale].unknownHost}</span>{session.remotePath && <code>{session.remotePath}</code>}</div>
        <div className="transfer-row-progress"><span>{labels.states[session.state]}</span><span>{session.dirty ? labels.unsent : labels.saved}</span>{session.error && <p className="sftp-dialog-error" role="alert">{errors[session.error]}</p>}
          {session.remoteTemporaries.length > 0 && <details><summary>{labels.temporaries} · {session.remoteTemporaries.length}</summary><p>{labels.cleanupHint}</p>{session.remoteTemporaries.map((temporary) => <div key={temporary.id}><code>{temporary.path}</code><span>{labels.temporaryStates[temporary.state]}</span></div>)}</details>}
          {session.recovered && session.error === "editor_residual" && <p>{labels.recoveryError}</p>}
        </div>
        <div className="transfer-row-actions">
          {session.remoteTemporaries.length > 0 && <button type="button" disabled={state.pending.includes(session.id)} onClick={() => {
            const binding = bindings.find((binding) => binding.hostId === session.hostId);
            if (!binding && session.hostId && onRetrySessionRequested && (session.recovered || session.error === "connection_lost")) { setRequested({ id: session.id, cleanup: true }); onRetrySessionRequested(session.hostId); }
            else void safely(() => remoteEditors.cleanup(session.id, binding));
          }}>{labels.cleanup}</button>}
          {session.state === "conflict" && <button type="button" disabled={state.pending.includes(session.id)} onClick={() => { setName(""); setDialog({ id: session.id, conflict: true }); }}>{labels.manage}</button>}
          {session.state === "residual" && !session.recovered && <button type="button" disabled={state.pending.includes(session.id)} onClick={() => {
            const binding = bindings.find((binding) => binding.hostId === session.hostId);
            if (!binding && session.error === "connection_lost" && session.hostId && onRetrySessionRequested) { setRequested({ id: session.id, cleanup: false }); onRetrySessionRequested(session.hostId); }
            else void safely(() => remoteEditors.retry(session.id, binding));
          }}>{labels.retry}</button>}
          <button type="button" disabled={state.pending.includes(session.id)} onClick={() => setDialog({ id: session.id, conflict: false })}>{labels.close}</button>
        </div>
      </article>)}</div>}
    </section>
    <Dialog.Root open={!!selected} onOpenChange={(open) => { if (!open && !busy) setDialog(null); }}>
      <Dialog.Portal><Dialog.Overlay className="sftp-dialog-overlay" /><Dialog.Content className="sftp-dialog" data-theme={theme} aria-busy={busy}>
        <Dialog.Title>{dialog?.conflict ? labels.conflict : labels.close}</Dialog.Title><Dialog.Description>{dialog?.conflict ? labels.conflictHint : labels.closeHint}</Dialog.Description>
        {selected && <>
          <p>{selected.name}</p>{selected.remotePath && <code>{selected.remotePath}</code>}
          {state.error && <p className="sftp-dialog-error" role="alert">{errors[state.error]}</p>}
          {dialog?.conflict && <>
            {!selected.replaceAllowed && <p>{labels.noReplace}</p>}
            <form onSubmit={(event) => { event.preventDefault(); if (name) void safely(async () => { await remoteEditors.resolve(selected.id, "keep_local_as_new", name); setDialog(null); }); }}>
              <label>{labels.name}<input required value={name} maxLength={255} onChange={(event) => setName(event.target.value)} autoComplete="off" /></label>
              <div className="sftp-dialog-actions"><button type="submit" className="sftp-confirm-action" disabled={busy || !name}>{labels.saveAs}</button></div>
            </form>
            <div className="sftp-dialog-actions">
              <button type="button" disabled={busy || !selected.replaceAllowed} onClick={() => void safely(async () => { await remoteEditors.resolve(selected.id, "replace_remote"); setDialog(null); })}>{labels.replace}</button>
              <button type="button" disabled={busy} onClick={() => void safely(async () => { await remoteEditors.resolve(selected.id, "keep_remote"); setDialog(null); })}>{labels.keepRemote}</button>
            </div>
          </>}
          <div className="sftp-dialog-actions">
            <button type="button" disabled={busy} onClick={() => setDialog(null)}>{labels.cancel}</button>
            <button type="button" disabled={busy} onClick={() => void safely(async () => { if (await remoteEditors.close(selected.id, "save_local_copy", locale)) setDialog(null); })}>{labels.exportCopy}</button>
            {!dialog?.conflict && <button type="button" disabled={busy} onClick={() => void safely(async () => { await remoteEditors.close(selected.id, "keep_for_recovery", locale); setDialog(null); })}>{labels.retain}</button>}
            {!dialog?.conflict && <button type="button" disabled={busy} onClick={() => void safely(async () => { await remoteEditors.close(selected.id, "discard", locale); setDialog(null); })}>{labels.discard}</button>}
          </div>
        </>}
      </Dialog.Content></Dialog.Portal>
    </Dialog.Root>
  </>;
}
