import { useCallback, useRef, useState, type ReactElement } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import type { TerminalTheme } from "../terminal/contracts";
import { AlertIcon } from "./icons";

export type ConfirmRequest = {
  title: string;
  body: string;
  /** Verb-first label naming the action and its object, e.g. "Delete prod-api-east". */
  confirm: string;
  cancel: string;
  /** Destructive actions use the danger key; anything else uses the amber primary. */
  tone?: "danger" | "primary";
  theme?: TerminalTheme;
};
type Pending = { request: ConfirmRequest; resolve: (accepted: boolean) => void };

/**
 * One in-app confirmation for irreversible actions. Focus starts on Cancel, so
 * Enter or Escape never destroys anything by accident.
 */
export function useConfirm(): [(request: ConfirmRequest) => Promise<boolean>, ReactElement | null] {
  const [pending, setPending] = useState<Pending | null>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const returnRef = useRef<HTMLElement | null>(null);
  const confirm = useCallback((request: ConfirmRequest) => new Promise<boolean>((resolve) => {
    returnRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setPending((current) => {
      current?.resolve(false);
      return { request, resolve };
    });
  }), []);
  const settle = (accepted: boolean): void => {
    pending?.resolve(accepted);
    setPending(null);
  };
  const request = pending?.request;
  const element = request ? <Dialog.Root open onOpenChange={(open) => { if (!open) settle(false); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="sftp-dialog-overlay" />
      <Dialog.Content className="sftp-dialog confirm-dialog" role="alertdialog" data-tone={request.tone ?? "danger"} data-theme={request.theme}
        onOpenAutoFocus={(event) => { event.preventDefault(); cancelRef.current?.focus(); }}
        onCloseAutoFocus={(event) => { event.preventDefault(); if (returnRef.current?.isConnected) returnRef.current.focus(); }}>
        <Dialog.Title><span className="dialog-mark" aria-hidden="true"><AlertIcon size={16} /></span>{request.title}</Dialog.Title>
        <Dialog.Description>{request.body}</Dialog.Description>
        <div className="sftp-dialog-actions">
          <button ref={cancelRef} className="ghost-button" type="button" onClick={() => settle(false)}>{request.cancel}</button>
          <button className={request.tone === "primary" ? "primary-action" : "danger-action"} type="button" onClick={() => settle(true)}>{request.confirm}</button>
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root> : null;
  return [confirm, element];
}
