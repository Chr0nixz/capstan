import * as Dialog from "@radix-ui/react-dialog";
import type { ReactElement } from "react";
import type { HostId, HostView } from "../terminal/contracts";
import { sshAuthority } from "./authority";

export function HostChoiceDialog({
  open,
  target,
  hosts,
  title,
  body,
  cancel,
  onChoose,
  onCancel,
}: {
  open: boolean;
  target: string;
  hosts: HostView[];
  title: string;
  body: string;
  cancel: string;
  onChoose: (hostId: HostId) => void;
  onCancel: () => void;
}): ReactElement | null {
  if (!open) return null;
  return <Dialog.Root open onOpenChange={(next) => { if (!next) onCancel(); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="sftp-dialog-overlay" />
      <Dialog.Content className="sftp-dialog launch-choice-dialog">
        <Dialog.Title>{title}</Dialog.Title>
        <Dialog.Description>{body} {target}</Dialog.Description>
        <div className="launch-choice-list">
          {hosts.map((host) => <button key={host.id} type="button" className="ghost-button" onClick={() => onChoose(host.id)}>
            <span>{host.name}</span><span className="mono">{sshAuthority(host.username, host.address, host.port)}</span>
          </button>)}
        </div>
        <div className="sftp-dialog-actions"><button type="button" className="ghost-button" onClick={onCancel}>{cancel}</button></div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
