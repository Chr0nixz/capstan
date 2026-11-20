import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactElement, type ReactNode } from "react";
import { copy } from "../i18n";
import { useWorkspace, type TerminalTab } from "../workspace";
import { BroadcastIcon, CheckIcon, MoreIcon, SnippetIcon, SplitDownIcon, SplitRightIcon, TunnelIcon } from "../ui/icons";
import { shortcutLabel } from "../window/platform";
import type { SplitDirection } from "./tree";

type Labels = typeof copy.en | typeof copy["zh-CN"];
type Item = { label: string; icon: ReactNode; keys?: string; checked?: boolean; run: () => void; keepFocus?: boolean } | "separator";

/**
 * The narrow-strip stand-in for the pane and sheet keys: one More key that
 * opens the same actions as a menu. CSS shows it only when the tab strip is
 * too narrow for the inline keys. It keeps the amber toggle look while
 * broadcast is on, so folding the keys never hides that state.
 */
export function TabToolsMenu({ tab, labels, onRequestFocus, onOpenSnippets, onOpenTunnels }: {
  tab: TerminalTab | undefined; labels: Labels; onRequestFocus: () => void;
  onOpenSnippets?: () => void; onOpenTunnels?: () => void;
}): ReactElement | null {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const restoreFocus = useRef(true);
  const menuId = useId();
  const [open, setOpen] = useState(false);
  const split = (direction: SplitDirection): void => {
    if (!tab) return;
    useWorkspace.getState().splitPane(tab.id, tab.focusedPaneId, direction);
    onRequestFocus();
  };
  const items: Item[] = [];
  if (tab) items.push(
    { label: labels.splitRight, icon: <SplitRightIcon size={14} />, keys: shortcutLabel("Alt+Shift+="), run: () => split("horizontal"), keepFocus: true },
    { label: labels.splitDown, icon: <SplitDownIcon size={14} />, keys: shortcutLabel("Alt+Shift+-"), run: () => split("vertical"), keepFocus: true },
    { label: labels.broadcastInput, icon: <BroadcastIcon size={14} />, checked: tab.broadcastEnabled, run: () => useWorkspace.getState().setBroadcast(tab.id, !tab.broadcastEnabled) },
  );
  if (onOpenSnippets && onOpenTunnels) {
    if (items.length) items.push("separator");
    items.push(
      { label: labels.openSnippets, icon: <SnippetIcon size={14} />, run: onOpenSnippets, keepFocus: true },
      { label: labels.openTunnels, icon: <TunnelIcon size={14} />, run: onOpenTunnels, keepFocus: true },
    );
  }
  const hasItems = items.length > 0;
  useEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const onToggle = (event: Event): void => {
      const next = (event as Event & { newState?: string }).newState === "open";
      setOpen(next);
      if (next) {
        const anchor = buttonRef.current?.getBoundingClientRect();
        if (anchor) {
          menu.style.top = `${Math.round(anchor.bottom + 4)}px`;
          menu.style.left = `${Math.round(Math.max(4, Math.min(anchor.right - menu.offsetWidth, window.innerWidth - menu.offsetWidth - 4)))}px`;
        }
        menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
      } else {
        if (restoreFocus.current && (document.activeElement === document.body || menu.contains(document.activeElement))) buttonRef.current?.focus();
        restoreFocus.current = true;
      }
    };
    menu.addEventListener("toggle", onToggle);
    return () => menu.removeEventListener("toggle", onToggle);
  }, [hasItems]);
  const close = (): void => {
    try { if (menuRef.current?.matches(":popover-open")) menuRef.current.hidePopover(); } catch { /* No popover support (tests). */ }
  };
  if (!hasItems) return null;
  const broadcasting = !!tab?.broadcastEnabled;
  const label = broadcasting ? labels.moreToolsBroadcasting : labels.moreTools;
  const keyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key === "Tab") { close(); return; }
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "ArrowDown" ? buttons[(index + 1) % buttons.length] : event.key === "ArrowUp" ? buttons[(index - 1 + buttons.length) % buttons.length]
      : event.key === "Home" ? buttons[0] : event.key === "End" ? buttons.at(-1) : undefined;
    if (!next) return;
    event.preventDefault();
    next.focus();
  };
  return <>
    <button ref={buttonRef} className="icon-button tab-tools-more" type="button" title={label} aria-label={label} aria-haspopup="menu"
      aria-expanded={open} aria-controls={menuId} popoverTarget={menuId} data-broadcast={broadcasting || undefined}><MoreIcon /></button>
    <div ref={menuRef} id={menuId} className="sftp-menu tab-tools-menu" popover="auto" role="menu" aria-label={labels.moreTools} onKeyDown={keyDown}>
      {items.map((item, index) => item === "separator" ? <div className="sftp-menu-separator" role="separator" key={`separator-${index}`} />
        : <button key={item.label} type="button" role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"} aria-checked={item.checked}
          onClick={() => { restoreFocus.current = !item.keepFocus; close(); item.run(); }}>
          {item.icon}<span className="sftp-menu-label" data-keys={item.keys}>{item.label}</span>{item.checked && <CheckIcon size={14} className="sftp-menu-check" />}
        </button>)}
    </div>
  </>;
}
