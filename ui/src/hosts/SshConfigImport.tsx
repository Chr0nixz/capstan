import * as Dialog from "@radix-ui/react-dialog";
import { useEffect, useState, type ReactElement } from "react";
import type { copy } from "../i18n";
import { sshAuthority } from "../launch/authority";
import { invokeTauri } from "../terminal/bridge";
import type { SshConfigHost, SshConfigPreview } from "../terminal/contracts";

type Labels = typeof copy.en | typeof copy["zh-CN"];
export type ImportSummary = { created: number; existing: number };

function errorCode(error: unknown): string | undefined {
  return typeof error === "object" && error !== null && "code" in error && typeof error.code === "string" ? error.code : undefined;
}

/** A host behind a jump host would only fail to connect, so it starts unchecked. */
function selectedByDefault(host: SshConfigHost): boolean {
  return !host.saved && !host.notes.includes("proxy_unsupported");
}

/** The hosts `index` is reached through, nearest first. */
function jumpsOf(hosts: readonly SshConfigHost[], index: number): number[] {
  const chain: number[] = [];
  let next = hosts.find((host) => host.index === index)?.jump ?? null;
  while (next !== null && next !== index && !chain.includes(next)) {
    chain.push(next);
    next = hosts.find((host) => host.index === next)?.jump ?? null;
  }
  return chain;
}

function fileName(path: string): string {
  return path.split(/[\\/]/u).pop() || path;
}

/**
 * Lists `~/.ssh/config` hosts for import. The backend keeps the parsed plan;
 * this dialog only sends the preview token, chosen indices and record names.
 * Key passphrases are asked for in native dialogs during import.
 */
export function SshConfigImportDialog({ open, labels, locale, onClose, onImported }: {
  open: boolean;
  labels: Labels;
  locale: string;
  onClose: () => void;
  onImported: (summary: ImportSummary) => void;
}): ReactElement | null {
  const [preview, setPreview] = useState<SshConfigPreview | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) { setPreview(null); setSelected(new Set()); setError(null); setBusy(false); return; }
    let current = true;
    invokeTauri("preview_ssh_config_import").then((next) => {
      if (!current) return;
      setPreview(next);
      setSelected(new Set(next.hosts.filter(selectedByDefault).map((host) => host.index)));
    }).catch(() => { if (current) setError(labels.operationFailed); });
    return () => { current = false; };
  }, [open, labels]);

  if (!open) return null;
  // A host imports with its jump hosts, so they are checked and unchecked together.
  const toggle = (index: number, checked: boolean): void => setSelected((current) => {
    const hosts = preview?.hosts ?? [];
    const next = new Set(current);
    if (checked) {
      for (const hop of [index, ...jumpsOf(hosts, index)]) {
        if (!hosts.find((host) => host.index === hop)?.saved) next.add(hop);
      }
    } else {
      next.delete(index);
      for (const host of hosts) if (jumpsOf(hosts, host.index).includes(index)) next.delete(host.index);
    }
    return next;
  });
  const aliasOf = (index: number): string => preview?.hosts.find((host) => host.index === index)?.alias ?? "";
  const submit = async (): Promise<void> => {
    if (!preview?.token || selected.size === 0) return;
    setBusy(true);
    setError(null);
    try {
      const summary = await invokeTauri("import_ssh_config", {
        token: preview.token, indices: [...selected].sort((left, right) => left - right), locale,
        credentialName: labels.interactiveCredentialName, groupName: labels.sshConfigGroup,
      });
      onImported(summary);
    } catch (cause) {
      const code = errorCode(cause);
      if (code === "canceled") setError(null);
      else if (code === "ssh_config_expired") setError(labels.sshConfigExpired);
      else if (code === "ssh_private_key_invalid") setError(labels.privateKeyInvalid);
      else if (code === "credential_file") setError(labels.credentialFileInvalid);
      else setError(labels.operationFailed);
    } finally {
      setBusy(false);
    }
  };
  const authLabel = (host: SshConfigHost): string => host.identityFile === null ? labels.sshConfigAsk
    : host.encrypted ? labels.sshConfigKeyLocked(fileName(host.identityFile)) : labels.sshConfigKey(fileName(host.identityFile));
  const status = preview === null ? (error ? null : labels.sshConfigLoading)
    : preview.token === null ? labels.sshConfigMissing
      : preview.hosts.length === 0 ? labels.sshConfigEmpty : null;

  return <Dialog.Root open onOpenChange={(next) => { if (!next && !busy) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="sftp-dialog-overlay" />
      <Dialog.Content className="sftp-dialog ssh-import-dialog" aria-busy={busy}>
        <Dialog.Title>{labels.importSshConfig}</Dialog.Title>
        <Dialog.Description>{labels.sshConfigIntro}</Dialog.Description>
        {status && <p className="quiet-copy" role="status">{status}</p>}
        {preview && preview.hosts.length > 0 && <ul className="ssh-import-list" aria-label={labels.importSshConfig}>
          {preview.hosts.map((host) => <li key={host.index}>
            <label className={"ssh-import-row" + (host.saved ? " saved" : "")}>
              <input type="checkbox" checked={selected.has(host.index)} disabled={host.saved || busy}
                onChange={(event) => toggle(host.index, event.currentTarget.checked)} />
              <span className="ssh-import-name">{host.alias}</span>
              <span className="ssh-import-target mono">{sshAuthority(host.username, host.address, host.port)}</span>
              <span className="ssh-import-detail">{host.saved ? labels.sshConfigSaved : authLabel(host)}
                {host.jump !== null && <span> · {labels.sshConfigVia(aliasOf(host.jump))}</span>}
                {host.notes.map((note) => <span key={note}> · {labels.sshConfigNotes[note]}</span>)}</span>
            </label>
          </li>)}
        </ul>}
        {preview && preview.skipped.length > 0 && <p className="quiet-copy">{labels.sshConfigSkipped(preview.skipped.length)}</p>}
        {error && <p className="field-error" role="alert">{error}</p>}
        <div className="sftp-dialog-actions">
          <button type="button" className="ghost-button" disabled={busy} onClick={onClose}>{labels.cancelEdit}</button>
          {preview?.token && preview.hosts.length > 0 && <button type="button" className="primary-action" disabled={busy || selected.size === 0}
            onClick={() => { void submit(); }}>{labels.sshConfigImport(selected.size)}</button>}
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
