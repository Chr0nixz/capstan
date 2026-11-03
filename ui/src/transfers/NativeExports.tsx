import { useEffect, useState, type ReactElement } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { invokeTauri, isTauriAvailable } from "../terminal/bridge";
import { sftpError } from "../sftp/controller";
import { sftpCopy } from "../sftp/copy";
import type { SftpErrorCode } from "../sftp/contracts";
import type { TerminalTheme } from "../terminal/contracts";
import { nativeExports, type NativeExport } from "./native";
import { transferCopy } from "./copy";
import { ChevronRightIcon, ExportIcon } from "../ui/icons";
export function NativeExports({ locale, theme }: { locale: "en" | "zh-CN"; theme: TerminalTheme }): ReactElement | null {
  const labels = transferCopy[locale];
  const [items, setItems] = useState<NativeExport[]>([]), [error, setError] = useState<SftpErrorCode | null>(null);
  const [selected, setSelected] = useState<NativeExport | null>(null), [pending, setPending] = useState(false);
  const [expanded, setExpanded] = useState(false);
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let active = true, reading = false;
    const refresh = async (): Promise<void> => { if (reading) return; reading = true; try { const items = await nativeExports(); if (active) setItems((previous) => [...previous.filter((old) => items.some((item) => item.token === old.token)).map((old) => items.find((item) => item.token === old.token)!), ...items.filter((item) => !previous.some((old) => old.token === item.token))]); } catch (cause) { if (active) setError(sftpError(cause)); } finally { reading = false; } };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 1000);
    return () => { active = false; window.clearInterval(timer); };
  }, []);
  if (!items.length) return null;
  return <section className="transfer-queue" data-theme={theme} aria-label={labels.exports}>
    <button type="button" className="transfer-queue-heading" aria-expanded={expanded} onClick={() => setExpanded((value) => !value)}><ChevronRightIcon size={14} /><span>{labels.exports}</span><span className="sr-only"> · </span><span className="count-badge">{items.length}</span></button>
    {expanded && <div className="transfer-rows"><p>{labels.exportRetention}</p>
    {error && <p role="alert" className="sftp-error">{sftpCopy[locale].errors[error]}</p>}
    {items.map((item) => <div className="native-export-row" key={item.token}><span className="transfer-direction" aria-hidden="true"><ExportIcon size={14} /></span><strong>{item.name}</strong><span>{labels.exportStates[item.state]}</span><button type="button" disabled={pending || item.state === "dragging"} onClick={() => setSelected(item)}>{labels.clearExport}</button></div>)}
    </div>}
    <Dialog.Root open={selected !== null} onOpenChange={(open) => { if (!pending && !open) setSelected(null); }}><Dialog.Portal><Dialog.Overlay className="sftp-dialog-overlay" /><Dialog.Content className="sftp-dialog" data-theme={theme}>
      <Dialog.Title>{labels.clearExport}</Dialog.Title><Dialog.Description>{labels.exportCleanup}</Dialog.Description>
      <p>{selected?.name}</p><div className="sftp-dialog-actions"><Dialog.Close asChild><button className="ghost-button" type="button" disabled={pending}>{labels.keepOpen}</button></Dialog.Close><button type="button" className="danger-action" disabled={pending} onClick={() => {
        if (!selected) return;
        setPending(true); setError(null);
        void invokeTauri("release_native_export", { token: selected.token }).then(() => { setItems((items) => items.filter((item) => item.token !== selected.token)); setSelected(null); }).catch((cause: unknown) => setError(sftpError(cause))).finally(() => setPending(false));
      }}>{labels.clearExportConfirm}</button></div>
    </Dialog.Content></Dialog.Portal></Dialog.Root>
  </section>;
}
