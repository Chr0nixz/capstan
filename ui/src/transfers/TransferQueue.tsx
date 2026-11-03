import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactElement } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { invokeTauri, isTauriAvailable, listenTauriEvent } from "../terminal/bridge";
import { sftpCopy } from "../sftp/copy";
import { editorCopy } from "../editors/copy";
import { sftpError } from "../sftp/controller";
import type { SftpErrorCode } from "../sftp/contracts";
import type { TerminalTheme } from "../terminal/contracts";
import { isFinal, type TransferRecovery } from "./contracts";
import { createTransferController } from "./controller";
import { transferCopy } from "./copy";
import { tauriTransferTransport } from "./transport";
import { selectRecoveredDestination, selectSourceForRetry } from "./selection";
import { NativeExports } from "./NativeExports";
import type { SftpSessionId } from "../sftp/contracts";
import type { HostId, TerminalSessionId } from "../terminal/contracts";
import { ChevronRightIcon, DownloadIcon, UploadIcon } from "../ui/icons";
import { formatBytes } from "../ui/format";

type RetrySession = { terminal: TerminalSessionId; hostId: HostId; session: SftpSessionId };

export { formatBytes };
function percent(done: string, total: string): number { return BigInt(total) === 0n ? 0 : Number(BigInt(done) * 1000n / BigInt(total)) / 10; }
function canRecover(state: TransferRecovery["state"]): boolean { return state === "awaiting_reconnect" || state === "available"; }
function canClean(item: TransferRecovery): boolean { return item.state === "cleanup_required" || item.direction === "download" && canRecover(item.state); }
function exitErrorCode(code: unknown): SftpErrorCode | "preferences" | null {
  if (code === undefined || code === null) return null;
  return ["settings_unavailable", "settings_invalid", "settings_changed"].includes(String(code)) ? "preferences" : sftpError({ code });
}
export function TransferQueue({ locale, hosts, production, sessions = [], theme = "graphite", onRetrySessionRequested }: { locale: "en" | "zh-CN"; hosts: Readonly<Record<string, string>>; production?: ReadonlySet<string>; sessions?: readonly RetrySession[]; theme?: TerminalTheme; onRetrySessionRequested?: (hostId: string) => void }): ReactElement | null {
  const labels = transferCopy[locale], errors = sftpCopy[locale].errors;
  const hostLabel = (hostId: string): ReactElement => <>{production?.has(hostId) && <span className="prod-chip">{sftpCopy[locale].production}</span>}{hosts[hostId] ?? labels.unknownHost}</>;
  const controller = useMemo(() => createTransferController(tauriTransferTransport()), []);
  const state = useSyncExternalStore(controller.store.subscribe, controller.store.getState, controller.store.getState);
  const [expanded, setExpanded] = useState(false), [selectedId, setSelectedId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [batchConsent, setBatchConsent] = useState<string | null>(null);
  const [retryError, setRetryError] = useState<SftpErrorCode | null>(null);
  const [retryPrompt, setRetryPrompt] = useState<string | null>(null);
  const [recoveryPrompt, setRecoveryPrompt] = useState<string | null>(null);
  const [cleanupPrompt, setCleanupPrompt] = useState<string | null>(null);
  const [exitOpen, setExitOpen] = useState(false), [exiting, setExiting] = useState(false);
  const [exitError, setExitError] = useState<SftpErrorCode | "preferences" | null>(null);
  const focusReturn = useRef<HTMLElement | null>(null);
  const attentionSeen = useRef(new Set<string>());
  // Finished rows the user cleared from view; the backend keeps its own records.
  const [cleared, setCleared] = useState<ReadonlySet<string>>(() => new Set());
  const selected = state.tasks.find((task) => task.id === selectedId);
  const consentKey = selected?.conflict && selected.batch ? `${selected.id}:${selected.conflict.revision}:${selected.batch.id}` : null;
  const batchAvailable = !!selected?.conflict?.batchEligible && !!selected.batch;
  const applyToBatch = batchAvailable && consentKey !== null && batchConsent === consentKey;
  // Paused work is unfinished but not moving, so it is counted on its own.
  const paused = state.tasks.filter((task) => task.state === "paused").length;
  const active = state.tasks.filter((task) => !isFinal(task.state) && task.state !== "paused").length;
  const finished = (task: (typeof state.tasks)[number]): boolean => task.state === "completed" || task.state === "canceled";
  const shownTasks = state.tasks.filter((task) => !(finished(task) && cleared.has(task.id)));
  const clearable = shownTasks.filter(finished);
  const retryTask = async (task: (typeof state.tasks)[number]): Promise<void> => {
    setRetryError(null); setRetryPrompt(null);
    const binding = sessions.find((candidate) => candidate.hostId === task.hostId);
    if (!binding) {
      setRetryPrompt(task.id);
      onRetrySessionRequested?.(task.hostId);
      return;
    }
    try {
      if (task.direction === "upload") {
        const slash = task.remotePath.lastIndexOf("/");
        const parent = slash <= 0 ? "/" : task.remotePath.slice(0, slash);
        const token = await selectSourceForRetry(binding.terminal, binding.session, parent, locale);
        if (!token) return;
        // The backend consumes the owner-bound token before starting the retry.
        // Release is only a best-effort fallback if the command rejects before
        // consuming it (for example, a stale task/session binding).
        let submitted = false;
        try {
          await controller.retryWithSelection(task.id, binding.terminal, binding.session, token);
          submitted = true;
        } finally {
          if (!submitted) await invokeTauri("release_transfer_selection", { token }).catch(() => undefined);
        }
      } else {
        await controller.retryOnSession(task.id, binding.terminal, binding.session);
      }
    } catch (cause) {
      setRetryError(sftpError(cause));
    }
  };
  const retryRecoveredUpload = async (id: string): Promise<void> => {
    const item = state.recovery.find((candidate) => candidate.id === id);
    if (!item || !canRecover(item.state)) return;
    const binding = sessions.find((candidate) => candidate.hostId === item.hostId);
    if (!binding) {
      setRecoveryPrompt(id);
      onRetrySessionRequested?.(item.hostId);
      return;
    }
    try {
      const slash = item.remotePath.lastIndexOf("/");
      const parent = slash <= 0 ? "/" : item.remotePath.slice(0, slash);
      const token = item.direction === "upload" ? await selectSourceForRetry(binding.terminal, binding.session, parent, locale) : await selectRecoveredDestination(binding.terminal, binding.session, item.id, locale);
      if (!token) return;
      let submitted = false;
      try {
        if (item.direction === "upload") await controller.retryRecoveredUpload(item.id, binding.terminal, binding.session, token);
        else await controller.retryRecoveredDownload(item.id, binding.terminal, binding.session, token);
        submitted = true;
      } finally {
        if (!submitted) await invokeTauri("release_transfer_selection", { token }).catch(() => undefined);
      }
      setRecoveryPrompt(null);
    } catch (cause) {
      setRetryError(sftpError(cause));
    }
  };
  const cleanupRecoveredUpload = async (id: string): Promise<void> => {
    const item = state.recovery.find((candidate) => candidate.id === id);
    if (item && !canClean(item)) return;
    const binding = item && sessions.find((candidate) => candidate.hostId === item.hostId);
    if (!item || !binding) { if (item) { setCleanupPrompt(id); onRetrySessionRequested?.(item.hostId); } return; }
    try {
      if (item.direction === "upload") await controller.cleanupRecoveredUpload(item.id, binding.terminal, binding.session);
      else {
        const token = await selectRecoveredDestination(binding.terminal, binding.session, item.id, locale);
        if (!token) return;
        try { await controller.cleanupRecoveredDownload(item.id, token); }
        finally { await invokeTauri("release_transfer_selection", { token }).catch(() => undefined); }
      }
      setCleanupPrompt(null);
    } catch (cause) { setRetryError(sftpError(cause)); }
  };
  // Work that needs a decision (conflicts, failures, recovered partials) opens
  // the queue and turns its badge red. Plain progress stays in the one-line
  // summary so the terminal keeps its height.
  const attention = [
    ...state.tasks.filter((task) => task.conflict || task.cleanupRequired || task.state === "failed" || task.state === "interrupted").map((task) => task.id),
    ...state.recovery.map((item) => `recovery:${item.id}`),
  ];
  const attentionKey = attention.join(",");
  useEffect(() => {
    const fresh = attention.filter((id) => !attentionSeen.current.has(id));
    attentionSeen.current = new Set(attention);
    if (fresh.length) setExpanded(true);
  }, [attentionKey]);
  const running = state.tasks.find((task) => task.state === "running" && task.remainingSeconds !== null);
  const summary = attention.length ? labels.summaryAttention(attention.length)
    : active || paused ? [
      active ? labels.summaryActive(active, running?.remainingSeconds != null ? labels.eta(running.remainingSeconds) : null) : "",
      paused ? labels.summaryPaused(paused) : "",
    ].filter(Boolean).join(" · ") : null;
  // A reconnect creates a new SFTP binding asynchronously.  Keep the retry
  // prompt actionable while the tab is opening, then bind the interrupted
  // task to the first verified session for that host.  Uploads still open a
  // fresh native picker in retryTask; no stale picker token is replayed.
  useEffect(() => {
    if (!retryPrompt) return;
    const task = state.tasks.find((candidate) => candidate.id === retryPrompt);
    if (!task) { setRetryPrompt(null); return; }
    const binding = sessions.find((candidate) => candidate.hostId === task.hostId);
    if (!binding) return;
    setRetryPrompt(null);
    void retryTask(task);
  }, [retryPrompt, sessions, state.tasks]);
  useEffect(() => {
    if (!recoveryPrompt) return;
    const item = state.recovery.find((candidate) => candidate.id === recoveryPrompt);
    if (!item || !canRecover(item.state)) { setRecoveryPrompt(null); return; }
    if (!sessions.some((candidate) => candidate.hostId === item.hostId)) return;
    setRecoveryPrompt(null);
    void retryRecoveredUpload(item.id);
  }, [recoveryPrompt, sessions, state.recovery]);
  useEffect(() => {
    if (cleanupPrompt && !state.recovery.some((item) => item.id === cleanupPrompt && canClean(item))) setCleanupPrompt(null);
  }, [cleanupPrompt, state.recovery]);
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let disposed = false, unlisten: (() => void) | undefined;
    void controller.refresh();
    const interval = window.setInterval(() => void controller.refresh(), 250);
    void listenTauriEvent<{ error?: string } | null>("transfer-exit-requested", (payload) => { setExitError(exitErrorCode(payload?.error)); setExitOpen(true); }).then((listener) => {
      if (disposed) listener(); else unlisten = listener;
    }).catch(() => undefined);
    return () => { disposed = true; unlisten?.(); window.clearInterval(interval); controller.dispose(); };
  }, [controller]);
  if (!isTauriAvailable()) return null;
  return <>
    <NativeExports locale={locale} theme={theme} />
    <section className="transfer-queue" data-theme={theme} data-active={active > 0 ? "true" : undefined} aria-label={labels.title}>
      <div className="transfer-queue-bar">
        {/* Announces only decisions the user must make, never routine progress. */}
        <span className="sr-only" role="status">{attention.length ? labels.summaryAttention(attention.length) : ""}</span>
        <button className="transfer-queue-heading" type="button" aria-expanded={expanded} aria-keyshortcuts="Control+Shift+J" onClick={() => setExpanded((value) => !value)}>
          <ChevronRightIcon size={14} /><span>{labels.title}</span><span className="sr-only"> · </span>
          <span className="count-badge" data-attention={attention.length ? "true" : undefined}>{attention.length || active + paused}</span>
          {summary && <><span className="sr-only"> · </span><span className="transfer-summary">{summary}</span></>}
        </button>
        {expanded && clearable.length > 0 && <button className="ghost-button transfer-clear" type="button" onClick={() => setCleared((current) => new Set([...current, ...clearable.map((task) => task.id)]))}>{labels.clearFinished}</button>}
      </div>
      {expanded && <div className="transfer-rows">
        {state.error && <p className="sftp-error" role="alert">{errors[state.error]}</p>}
        {retryError && <p className="sftp-error" role="alert">{errors[retryError]}</p>}
        {retryPrompt && <div className="sftp-error transfer-retry-help" role="alert">
          <span>{transferCopy[locale].retryConnection}</span>
          <button type="button" onClick={() => onRetrySessionRequested?.(state.tasks.find((task) => task.id === retryPrompt)?.hostId ?? "")}>{transferCopy[locale].openConnection}</button>
          <button type="button" onClick={() => { const task = state.tasks.find((candidate) => candidate.id === retryPrompt); if (task) void retryTask(task); }}>{labels.retry}</button>
        </div>}
        {state.recoveryIncomplete && <p className="sftp-error" role="alert">{labels.recoveryIncomplete}</p>}
        {state.recovery.length > 0 && <section className="transfer-recovery" aria-label={labels.residuals}>
          <h3>{labels.residuals}</h3>
          {recoveryPrompt && <div className="sftp-error transfer-retry-help" role="alert">
            <span>{labels.retryConnection}</span>
            <button type="button" onClick={() => { const item = state.recovery.find((candidate) => candidate.id === recoveryPrompt); if (item) onRetrySessionRequested?.(item.hostId); }}>{labels.openConnection}</button>
            <button type="button" onClick={() => void retryRecoveredUpload(recoveryPrompt)}>{labels.resumeRecovered}</button>
          </div>}
          {state.recovery.map((item) => <article className="transfer-row" data-state={item.state} key={`recovery:${item.id}`}>
            <span className="transfer-direction" aria-hidden="true">{item.direction === "upload" ? <UploadIcon size={14} /> : <DownloadIcon size={14} />}</span>
            <div className="transfer-row-context"><strong>{item.name}</strong><span>{item.direction === "upload" ? labels.upload : labels.download} · {hostLabel(item.hostId)}</span><code>{item.remotePath}</code></div>
            <div className="transfer-row-progress"><span>{labels.recoveryState[item.state]}</span><span>{item.partialSize === null ? "—" : formatBytes(item.partialSize)} / {formatBytes(item.total)}</span>
              {item.localPartial && <p>{labels.localPartial}</p>}
              {item.state === "manual_inspection" && <p>{labels.manualRecovery}</p>}
              {item.direction === "download" && canRecover(item.state) && <p>{labels.downloadRecoveryManual}</p>}
              {item.direction === "upload" && canRecover(item.state) && <p>{labels.uploadRecoveryManual}</p>}
            </div>
            {(canRecover(item.state) || canClean(item)) && <div className="transfer-row-actions">
              {canRecover(item.state) && <button type="button" disabled={state.pending.includes(item.id)} onClick={() => void retryRecoveredUpload(item.id)}>{item.direction === "upload" ? labels.resumeRecovered : labels.resumeDownload}</button>}
              {canClean(item) && <button type="button" disabled={state.pending.includes(item.id)} onClick={() => setCleanupPrompt(item.id)}>{labels.removePartial}</button>}
            </div>}
          </article>)}
        </section>}
        {!shownTasks.length && <p className="sftp-status">{labels.empty}</p>}
        {shownTasks.map((task) => <article className="transfer-row" data-state={task.state} key={task.id} aria-label={`${task.name} · ${labels.states[task.state]}`}>
          <span className="transfer-direction" aria-hidden="true">{task.direction === "upload" ? <UploadIcon size={14} /> : <DownloadIcon size={14} />}</span>
          <div className="transfer-row-context"><strong>{task.name}</strong><span>{task.direction === "upload" ? labels.upload : labels.download} · {hostLabel(task.hostId)}</span><code>{task.remotePath}</code></div>
          <div className="transfer-row-progress"><span>{labels.states[task.state]}</span><span>{formatBytes(task.processed)} / {formatBytes(task.total)}</span>
            {!isFinal(task.state) && <progress aria-label={task.name} max="100" value={percent(task.processed, task.total)} />}
            {task.state === "running" && <span>{formatBytes(task.bytesPerSecond)}/s{task.remainingSeconds !== null ? ` · ${labels.eta(task.remainingSeconds)}` : ""}</span>}
            {task.error && <p className="sftp-dialog-error">{errors[task.error]}</p>}
            {task.cleanupRequired && <p className="sftp-dialog-error">{labels.cleanup}</p>}
          </div>
          <div className="transfer-row-actions">
            {task.conflict && <button type="button" disabled={state.pending.includes(task.id)} onClick={(event) => { focusReturn.current = event.currentTarget; setSelectedId(task.id); setName(task.name); setBatchConsent(null); }}>{labels.conflict}</button>}
            {!isFinal(task.state) && task.state !== "committing" && <>
              {(task.state === "running" || task.state === "resuming") && <button type="button" disabled={state.pending.includes(task.id)} onClick={() => void controller.pause(task.id)}>{labels.pause}</button>}
              {task.state === "paused" && <button type="button" disabled={state.pending.includes(task.id)} onClick={() => void controller.resume(task.id)}>{labels.resume}</button>}
              <button type="button" disabled={state.pending.includes(task.id)} onClick={() => void controller.cancel(task.id)}>{labels.cancel}</button>
            </>}
            {((task.state === "failed" && !task.cleanupRequired) || task.state === "interrupted") && !state.recovery.some((item) => item.id === task.id && (item.state === "cleanup_required" || item.state === "manual_inspection")) && <button type="button" disabled={state.pending.includes(task.id)} onClick={() => void retryTask(task)}>{labels.retry}</button>}
          </div>
        </article>)}
      </div>}
    </section>
    <Dialog.Root open={!!cleanupPrompt} onOpenChange={(open) => { if (!open) setCleanupPrompt(null); }}>
      <Dialog.Portal><Dialog.Overlay className="sftp-dialog-overlay" /><Dialog.Content className="sftp-dialog" data-theme={theme}>
        <Dialog.Title>{labels.removePartialTitle}</Dialog.Title><Dialog.Description>{labels.removePartialDescription}</Dialog.Description>
        <div className="sftp-dialog-actions"><Dialog.Close asChild><button className="ghost-button" type="button">{labels.keepOpen}</button></Dialog.Close>
          <button type="button" className="danger-action" disabled={!cleanupPrompt || state.pending.includes(cleanupPrompt)} onClick={() => { if (cleanupPrompt) void cleanupRecoveredUpload(cleanupPrompt); }}>{labels.removePartialConfirm}</button>
        </div>
      </Dialog.Content></Dialog.Portal>
    </Dialog.Root>
    <Dialog.Root open={!!selected?.conflict} onOpenChange={(open) => { if (!open) setSelectedId(null); }}>
      <Dialog.Portal><Dialog.Overlay className="sftp-dialog-overlay" /><Dialog.Content className="sftp-dialog" data-theme={theme} onCloseAutoFocus={(event) => { event.preventDefault(); focusReturn.current?.focus(); }}>
        <Dialog.Title>{labels.conflict}</Dialog.Title><Dialog.Description>{labels.changed}</Dialog.Description>
        {selected?.conflict && <>
          <p className="confirm-target"><span className="confirm-host">{hostLabel(selected.hostId)} · {selected.name}</span><code>{selected.remotePath}</code></p>
          {selected.batch && <p>{labels.batch(selected.batch.name, selected.batch.files)}</p>}
          <p className="transfer-sizes"><span>{labels.existingSize(selected.conflict.target?.size ? formatBytes(selected.conflict.target.size) : "—")}</span><span>{labels.incomingSize(formatBytes(selected.total))}</span></p>
          {!selected.conflict.replaceAllowed && <p>{selected.conflict.target?.kind !== "file" && selected.conflict.target ? labels.typeConflict : labels.noReplace}</p>}
          {state.error && <p className="sftp-dialog-error" role="alert">{errors[state.error]}</p>}
          <form onSubmit={(event) => { event.preventDefault(); if (name) void controller.resolve(selected, { kind: "rename", name }); }}>
            <label>{labels.name}<input value={name} onChange={(event) => setName(event.target.value)} required autoComplete="off" /></label>
            {batchAvailable && selected.batch && <div className="transfer-batch-scope">
              <label className="transfer-batch-choice"><input type="checkbox" checked={applyToBatch} disabled={state.pending.includes(selected.id)} onChange={(event) => setBatchConsent(event.target.checked ? consentKey : null)} /><span>{labels.batchScope(selected.batch.conflicts)}</span></label>
              <p>{labels.batchHelp}</p>
            </div>}
            <div className="sftp-dialog-actions">
              <button className="ghost-button" type="button" disabled={state.pending.includes(selected.id)} onClick={() => void controller.resolve(selected, { kind: applyToBatch ? "skip_batch" : "skip" })}>{applyToBatch ? labels.skipBatch : labels.skip}</button>
              <button className="danger-action" type="button" disabled={state.pending.includes(selected.id) || !selected.conflict.replaceAllowed} onClick={() => void controller.resolve(selected, { kind: applyToBatch ? "replace_batch" : "replace" })}>{applyToBatch ? labels.replaceBatch : labels.replace}</button>
              <button className="primary-action" type="submit" disabled={state.pending.includes(selected.id) || !name}>{labels.rename}</button>
            </div>
          </form>
        </>}
      </Dialog.Content></Dialog.Portal>
    </Dialog.Root>
    <Dialog.Root open={exitOpen} onOpenChange={(open) => { if (!exiting) setExitOpen(open); }}>
      <Dialog.Portal><Dialog.Overlay className="sftp-dialog-overlay" /><Dialog.Content className="sftp-dialog" data-theme={theme} aria-busy={exiting}>
        <Dialog.Title>{exitError === "preferences" ? sftpCopy[locale].preferencesExitTitle : exitError === "editor_residual" ? editorCopy[locale].title : labels.exitTitle}</Dialog.Title><Dialog.Description>{exitError === "preferences" ? sftpCopy[locale].preferencesExitDescription : exitError === "editor_residual" ? errors.editor_residual : labels.exitDescription}</Dialog.Description>
        {exitError && exitError !== "editor_residual" && <p className="sftp-dialog-error" role="alert">{exitError === "preferences" ? sftpCopy[locale].preferencesSaveFailed : errors[exitError]}</p>}
        <div className="sftp-dialog-actions"><Dialog.Close asChild><button type="button" disabled={exiting}>{labels.keepOpen}</button></Dialog.Close>
          <button type="button" disabled={exiting} onClick={() => { setExiting(true); void controller.exit().catch((cause: unknown) => {
            const code = typeof cause === "object" && cause !== null && "code" in cause ? cause.code : undefined;
            setExitError(exitErrorCode(code) ?? "remote_io"); setExiting(false);
          }); }}>{exitError === "preferences" || exitError === "editor_residual" ? sftpCopy[locale].preferencesRetryExit : labels.exit}</button>
        </div>
      </Dialog.Content></Dialog.Portal>
    </Dialog.Root>
  </>;
}
