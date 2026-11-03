import { useEffect, useState, type ReactElement } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import type { TerminalTheme } from "../terminal/contracts";
import type { TreeDecision, TreePlan } from "./tree";
import { transferCopy } from "./copy";
export function TreeTransferDialog({ plan, hostLabel, locale, theme, pending, error, onDecision, onClose }: { plan: TreePlan; hostLabel: string; locale: "en" | "zh-CN"; theme: TerminalTheme; pending: boolean; error: string | null; onDecision: (decisions: TreeDecision[]) => void; onClose: () => void }): ReactElement {
  const labels = transferCopy[locale], first = plan.conflicts[0];
  const mergeable = plan.conflicts.filter((conflict) => conflict.kind === "directory_exists").slice(0, 50);
  const [name, setName] = useState("");
  useEffect(() => { setName(first?.relative.split("/").at(-1) ?? ""); }, [plan.token, plan.revision, first?.entry]);
  return <Dialog.Root open onOpenChange={(open) => { if (!open && !pending) onClose(); }}><Dialog.Portal><Dialog.Overlay className="sftp-dialog-overlay" /><Dialog.Content className="sftp-dialog" data-theme={theme} aria-busy={pending}>
    <Dialog.Title>{labels.treeTitle}</Dialog.Title><Dialog.Description>{labels.treeHelp}</Dialog.Description><p>{hostLabel} · {plan.name} · {plan.conflicts.length}</p>
    {first && <><p>{labels.treeKinds[first.kind]}</p><code>{first.source}</code><p>{first.relative}</p></>}
    {mergeable.length > 1 && <ul className="transfer-directory-review">{mergeable.map((item) => <li key={item.entry}><code>{item.relative}</code></li>)}</ul>}
    {error && <p className="sftp-dialog-error" role="alert">{error}</p>}
    <form onSubmit={(event) => { event.preventDefault(); if (first && name) onDecision([{ entry: first.entry, decision: { kind: "rename", name } }]); }}>
      {first && <label>{labels.name}<input value={name} disabled={pending} required autoComplete="off" onChange={(event) => setName(event.target.value)} /></label>}
      <div className="sftp-dialog-actions"><button type="button" disabled={pending} onClick={onClose}>{labels.cancel}</button>
        {first && <><button type="button" disabled={pending} onClick={() => onDecision([{ entry: first.entry, decision: { kind: "skip" } }])}>{labels.skipSubtree}</button>
          {first.kind === "directory_exists" && <button type="button" disabled={pending} onClick={() => onDecision([{ entry: first.entry, decision: { kind: "merge" } }])}>{labels.mergeDirectory}</button>}
          {mergeable.length > 1 && <button type="button" disabled={pending} onClick={() => onDecision(mergeable.map((conflict) => ({ entry: conflict.entry, decision: { kind: "merge" } })))}>{labels.mergeDirectories} · {mergeable.length}</button>}
          <button type="submit" disabled={pending || !name}>{labels.rename}</button></>}
        {!first && <button type="button" disabled={pending} onClick={() => onDecision([])}>{labels.retry}</button>}
      </div>
    </form>
  </Dialog.Content></Dialog.Portal></Dialog.Root>;
}
