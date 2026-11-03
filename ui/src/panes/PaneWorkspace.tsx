import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { createPortal } from "react-dom";
import * as Dialog from "@radix-ui/react-dialog";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { copy } from "../i18n";
import { TerminalPane } from "../terminal/TerminalPane";
import { paneStatus, useWorkspace, type TerminalTab } from "../workspace";
import { BroadcastIcon, CloseIcon, SplitDownIcon, SplitRightIcon } from "../ui/icons";
import type { HostId, TerminalTheme } from "../terminal/contracts";
import type { BroadcastConfirmation, PaneId, PaneNode } from "./contracts";
import type { InputDeliveryResult } from "./input";
import { paneLeaves, splitId, type SplitDirection } from "./tree";

type Labels = typeof copy.en | typeof copy["zh-CN"];
type Props = {
  tab: TerminalTab; visible: boolean; focusRequest: number; labels: Labels; name: string;
  productionHosts: ReadonlySet<HostId>; restartTheme?: TerminalTheme;
  onRequestFocus: () => void;
};

export function PaneTools({ tab, labels, onRequestFocus }: Pick<Props, "tab" | "labels" | "onRequestFocus">): ReactElement {
  const split = (direction: SplitDirection): void => {
    useWorkspace.getState().splitPane(tab.id, tab.focusedPaneId, direction);
    onRequestFocus();
  };
  return <>
    <button className="icon-button" type="button" title={labels.shortcut(labels.splitRight, "Alt+Shift+=")} aria-label={labels.splitRight} aria-keyshortcuts="Alt+Shift+Equal" onClick={() => split("horizontal")}><SplitRightIcon /></button>
    <button className="icon-button" type="button" title={labels.shortcut(labels.splitDown, "Alt+Shift+-")} aria-label={labels.splitDown} aria-keyshortcuts="Alt+Shift+Minus" onClick={() => split("vertical")}><SplitDownIcon /></button>
    <button className="icon-button broadcast-toggle" type="button" title={labels.broadcastInputHint} aria-label={labels.broadcastInput} aria-pressed={tab.broadcastEnabled}
      onClick={() => useWorkspace.getState().setBroadcast(tab.id, !tab.broadcastEnabled)}>
      <BroadcastIcon />
    </button>
  </>;
}

type Pending = { ids: PaneId[]; names: string[]; reason: BroadcastConfirmation; resolve: (accepted: boolean) => void };

export function PaneWorkspace({ tab, visible, focusRequest, labels, name, productionHosts, restartTheme, onRequestFocus }: Props): ReactElement {
  const leaves = paneLeaves(tab.root);
  const mounts = useRef(new Map<PaneId, HTMLDivElement>());
  const paneNumbers = useRef(new Map<PaneId, number>());
  const nextPaneNumber = useRef(1);
  for (const pane of leaves) if (!paneNumbers.current.has(pane.paneId)) paneNumbers.current.set(pane.paneId, nextPaneNumber.current++);
  const [delivery, setDelivery] = useState<InputDeliveryResult | null>(null);
  const reportDelivery = useCallback((result: InputDeliveryResult): void => {
    // Successful keystrokes do not rerender the pane tree or dismiss a failure
    // that the user still needs to inspect.
    if (result.failedTargets.length > 0) setDelivery(result);
  }, []);
  const [pending, setPending] = useState<Pending | null>(null);
  const pendingRef = useRef<Pending | null>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const returnRef = useRef<HTMLElement | null>(null);
  // Keep a closed target's number available for an in-flight failure report.
  const paneName = (id: PaneId): string => labels.paneName(name, paneNumbers.current.get(id) ?? 0);
  const settle = (accepted: boolean): void => {
    const current = pendingRef.current;
    // A confirmed target set stays armed until the selection changes, so later
    // single commands go straight through; several lines always ask again.
    if (accepted && current) useWorkspace.getState().armBroadcast(tab.id, current.ids);
    current?.resolve(accepted);
    pendingRef.current = null;
    setPending(null);
  };
  useEffect(() => {
    if (!visible) {
      pendingRef.current?.resolve(false);
      pendingRef.current = null;
      setPending(null);
    }
  }, [visible]);
  useEffect(() => () => { pendingRef.current?.resolve(false); }, []);
  useEffect(() => {
    const ids = new Set(leaves.map((pane) => pane.paneId));
    for (const id of mounts.current.keys()) if (!ids.has(id)) mounts.current.delete(id);
  });
  const confirmBroadcast = (ids: readonly PaneId[], reason: BroadcastConfirmation): Promise<boolean> => new Promise((resolve) => {
    pendingRef.current?.resolve(false);
    returnRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const next = { ids: [...ids], names: ids.map(paneName), reason, resolve };
    pendingRef.current = next;
    setPending(next);
  });
  const mountFor = (id: PaneId): HTMLDivElement => {
    let mount = mounts.current.get(id);
    if (!mount) {
      mount = document.createElement("div");
      mount.className = "pane-mount";
      mounts.current.set(id, mount);
    }
    return mount;
  };
  const layout = (node: PaneNode): ReactElement => {
    if (node.kind === "pane") return <div className="pane-slot" ref={(slot) => {
      const mount = mountFor(node.paneId);
      if (slot && mount.parentElement !== slot) slot.appendChild(mount);
    }} />;
    const id = splitId(node);
    return <PanelGroup key={id} direction={node.direction} className="pane-split" keyboardResizeBy={5}
      onLayout={(sizes) => useWorkspace.getState().resizePaneSplit(tab.id, id, sizes[0] / 100)}>
      <Panel id={`${id}/first`} order={0} defaultSize={node.ratio * 100} minSize={5}>{layout(node.first)}</Panel>
      <PanelResizeHandle className="pane-resize-handle" aria-label={node.direction === "horizontal" ? labels.resizePanesHorizontal : labels.resizePanesVertical} />
      <Panel id={`${id}/second`} order={1} defaultSize={(1 - node.ratio) * 100} minSize={5}>{layout(node.second)}</Panel>
    </PanelGroup>;
  };
  const header = leaves.length > 1 || tab.broadcastEnabled;
  const armed = tab.broadcastArmedPaneIds.length > 0;
  return <div className="pane-workspace">
    {tab.broadcastEnabled && <div className="broadcast-status" role="status" data-armed={armed || undefined}>
      <BroadcastIcon size={14} />
      <span>{tab.broadcastPaneIds.length === 0 ? labels.broadcastNoTargets
        : <><strong>{labels.broadcastTargets(tab.broadcastPaneIds.length)}</strong> · {armed ? labels.broadcastArmed : labels.broadcastArmHint}</>}</span>
      <button className="ghost-button" type="button" onClick={() => useWorkspace.getState().setBroadcast(tab.id, false)}>{labels.stopBroadcast}</button>
    </div>}
    {delivery && delivery.failedTargets.length > 0 && <div className="pane-input-error" role="alert">
      <span>{labels.paneInputFailed(delivery.deliveredPaneIds.length)} {delivery.failedTargets.map(({ paneId, errorCode }) =>
        `${paneName(paneId)} (${errorCode === "input_queue_full" || errorCode === "queue_full" ? labels.paneInputQueueFull : errorCode === "session_closed" || errorCode === "not_found" ? labels.paneInputClosed : labels.paneInputUnknown})`).join(labels.paneListSeparator)}</span>
      <button className="icon-button" type="button" title={labels.dismiss} aria-label={labels.dismiss} onClick={() => setDelivery(null)}><CloseIcon size={12} /></button>
    </div>}
    <div className="pane-layout">{layout(tab.root)}</div>
    {/* Portal owners are flat and keyed by PaneId. Reparenting their stable DOM
        mounts into a different split never remounts xterm or its controller. */}
    {leaves.map((pane) => {
      const production = pane.launch.kind === "ssh" && productionHosts.has(pane.launch.host_id);
      const focused = tab.focusedPaneId === pane.paneId;
      const selected = tab.broadcastEnabled && tab.broadcastPaneIds.includes(pane.paneId);
      const theme = tab.paneStates[pane.paneId]?.themeOverride ?? (pane.launch.kind === "ssh" ? pane.launch.theme : "graphite");
      const label = paneName(pane.paneId);
      const focus = (): void => { useWorkspace.getState().focusPane(tab.id, pane.paneId); };
      const close = (): void => { useWorkspace.getState().closePane(tab.id, pane.paneId); onRequestFocus(); };
      return createPortal(<section className="workspace-pane" data-pane-id={pane.paneId} data-focused={focused || undefined} data-broadcast={selected || undefined}
        data-theme={theme} aria-label={label} onFocusCapture={focus} onPointerDownCapture={focus}>
        {header && <div className="workspace-pane-heading">
          {tab.broadcastEnabled && <input type="checkbox" aria-label={labels.selectBroadcastPane(label)} title={labels.selectBroadcastPane(label)} checked={tab.broadcastPaneIds.includes(pane.paneId)}
            onChange={() => useWorkspace.getState().toggleBroadcastPane(tab.id, pane.paneId)} />}
          <button className="pane-focus" type="button" title={label} onClick={() => { focus(); onRequestFocus(); }}>
            <span className="led" data-state={paneStatus(tab, pane.paneId)} aria-hidden="true" /><span>{label}</span>
          </button>
          {selected && <span className="broadcast-chip"><BroadcastIcon size={12} />{labels.broadcastChip}</span>}
          {production && <span className="prod-chip">{labels.production}</span>}
          <button className="icon-button" type="button" title={labels.closePane} aria-label={labels.closePane + ": " + label} onClick={close}><CloseIcon size={12} /></button>
        </div>}
        <div className="workspace-pane-terminal"><TerminalPane tabId={tab.id} paneId={pane.paneId} launch={pane.launch} visible={visible} focused={focused} themeOverride={theme}
          labels={labels} focusRequest={focusRequest} label={pane.launch.kind === "ssh" ? labels.sshTerminal(label) : label} production={production}
          confirmBroadcast={confirmBroadcast} onInputResult={reportDelivery}
          onRestart={() => { useWorkspace.getState().restartPane(tab.id, pane.paneId, restartTheme); onRequestFocus(); }} onClose={close} /></div>
      </section>, mountFor(pane.paneId), pane.paneId);
    })}
    {pending && visible && <Dialog.Root open onOpenChange={(open) => { if (!open) settle(false); }}><Dialog.Portal>
      <Dialog.Overlay className="sftp-dialog-overlay" />
      <Dialog.Content className="sftp-dialog confirm-dialog broadcast-confirm" role="alertdialog" data-tone={pending.reason === "arm" ? "primary" : "danger"}
        onOpenAutoFocus={(event) => { event.preventDefault(); cancelRef.current?.focus(); }}
        onCloseAutoFocus={(event) => { event.preventDefault(); if (returnRef.current?.isConnected) returnRef.current.focus(); }}>
        <Dialog.Title><span className="dialog-mark" aria-hidden="true"><BroadcastIcon size={16} /></span>
          {pending.reason === "arm" ? labels.confirmBroadcastArmTitle(pending.names.length) : labels.confirmBroadcastTitle}</Dialog.Title>
        <Dialog.Description>{pending.reason === "arm" ? labels.confirmBroadcastArmMessage(pending.names.length) : labels.confirmBroadcastMessage(pending.names.length)}</Dialog.Description>
        <ul className="broadcast-confirm-targets">{pending.names.map((target) => <li key={target}>{target}</li>)}</ul>
        <div className="sftp-dialog-actions">
          <button ref={cancelRef} className="ghost-button" type="button" onClick={() => settle(false)}>{labels.cancelEdit}</button>
          <button className={pending.reason === "arm" ? "primary-action" : "danger-action"} type="button" onClick={() => settle(true)}>{labels.confirmBroadcastSend(pending.names.length)}</button>
        </div>
      </Dialog.Content>
    </Dialog.Portal></Dialog.Root>}
  </div>;
}
