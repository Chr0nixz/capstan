import { useEffect, useState, type ReactElement } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import type { TerminalSessionId, TerminalTheme } from "../terminal/contracts";
import type { RemoteEntryId, SftpErrorCode, SftpSessionId } from "./contracts";
import { sftpError } from "./controller";
import { sftpCopy } from "./copy";
import { applyTreeDelete, cancelTreeDelete, prepareTreeDelete, type DeleteTreeReport, type DeleteTreeSummary } from "./treeDelete";
import { treeDeleteCopy } from "./treeDeleteCopy";
import { TrashIcon } from "../ui/icons";

type Props = { terminal: TerminalSessionId; session: SftpSessionId; entry: RemoteEntryId; hostLabel: string; production?: boolean; locale: keyof typeof sftpCopy; theme: TerminalTheme; onClose: () => void; onChanged: () => void };
export function TreeDeleteDialog({ terminal, session, entry, hostLabel, production = false, locale, theme, onClose, onChanged }: Props): ReactElement {
  const copy = treeDeleteCopy[locale], labels = sftpCopy[locale];
  const [plan, setPlan] = useState<DeleteTreeSummary | null>(null);
  const [error, setError] = useState<SftpErrorCode | null>(null);
  const [running, setRunning] = useState(false), [stopping, setStopping] = useState(false), [attempted, setAttempted] = useState(false);
  const [report, setReport] = useState<DeleteTreeReport | null>(null), [page, setPage] = useState(0);
  useEffect(() => {
    let active = true, token: string | undefined;
    void prepareTreeDelete(terminal, session, entry).then((plan) => {
      token = plan.token;
      if (active) setPlan(plan); else void cancelTreeDelete(terminal, session, token).catch(() => undefined);
    }).catch((cause: unknown) => { if (active) setError(sftpError(cause)); });
    return () => { active = false; if (token) void cancelTreeDelete(terminal, session, token).catch(() => undefined); };
  }, [terminal, session, entry]);
  const stop = (): void => {
    if (!plan || stopping) return;
    setStopping(true);
    void cancelTreeDelete(terminal, session, plan.token).catch((cause: unknown) => { setError(sftpError(cause)); setStopping(false); });
  };
  const close = (): void => { if (running) stop(); else onClose(); };
  const apply = async (): Promise<void> => {
    if (!plan || attempted) return;
    setAttempted(true); setRunning(true); setError(null);
    try { setReport(await applyTreeDelete(terminal, session, plan.token)); }
    catch (cause) { setError(sftpError(cause)); }
    finally { setRunning(false); onChanged(); }
  };
  const removed = report?.entries.filter((entry) => entry.outcome === "removed").length ?? 0;
  return <Dialog.Root open onOpenChange={(open) => { if (!open) close(); }}><Dialog.Portal>
    <Dialog.Overlay className="sftp-dialog-overlay" />
    <Dialog.Content className="sftp-dialog confirm-dialog" role="alertdialog" data-tone="danger" data-theme={theme}>
      <Dialog.Title><span className="dialog-mark" aria-hidden="true"><TrashIcon size={16} /></span>{copy.title}</Dialog.Title>
      <Dialog.Description>{copy.scope}</Dialog.Description>
      <p className="confirm-target">{production && <span className="prod-chip">{labels.production}</span>}<span className="confirm-host">{hostLabel}</span>{plan && <code>{plan.path}</code>}</p>
      {plan ? <p>{copy.summary(plan.files, plan.directories, plan.symlinks)}</p> : !error && <p role="status">{copy.scanning}</p>}
      {error && <p className="sftp-dialog-error" role="alert">{labels.errors[error]}</p>}
      {running && <p role="status">{stopping ? copy.stopping : copy.deleting}</p>}
      {report && <>
        <p role="status">{copy.result(removed, report.entries.length - removed)}</p>
        <ul className="transfer-tree-directories">{report.entries.slice(page * 50, (page + 1) * 50).map((entry) => <li key={entry.path}><code>{entry.path}</code><span>{copy.outcomes[entry.outcome]}{entry.error && ` · ${labels.errors[entry.error]}`}</span></li>)}</ul>
        {report.entries.length > 50 && <nav className="sftp-dialog-actions"><button type="button" disabled={page === 0} onClick={() => setPage((value) => value - 1)}>{copy.previous}</button><button type="button" disabled={(page + 1) * 50 >= report.entries.length} onClick={() => setPage((value) => value + 1)}>{copy.next}</button></nav>}
      </>}
      <div className="sftp-dialog-actions"><button className="ghost-button" type="button" disabled={running && stopping} onClick={close}>{running ? copy.stop : copy.close}</button>
        {!attempted && <button type="button" className="danger-action" disabled={!plan || !!error} onClick={() => void apply()}>{copy.apply}</button>}
      </div>
    </Dialog.Content>
  </Dialog.Portal></Dialog.Root>;
}
