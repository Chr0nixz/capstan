import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactElement } from "react";
import { ChevronDownIcon } from "../ui/icons";
import type { TabStatus } from "../workspace";

/** `signal` is null for a healthy local shell, which stays dark as it does on the tab strip. */
export type OverflowTab = { id: string; name: string; signal: TabStatus | null; production: boolean; selected: boolean };

/**
 * Every tab in one list, for when the strip scrolls some of them out of view.
 * CSS shows the key only while the tab list overflows.
 */
export function TabOverflowMenu({ tabs, label, productionLabel, onSelect }: {
  tabs: readonly OverflowTab[]; label: string; productionLabel: string; onSelect: (id: string) => void;
}): ReactElement {
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const [open, setOpen] = useState(false);
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
        (menu.querySelector<HTMLButtonElement>('[aria-current="page"]') ?? menu.querySelector<HTMLButtonElement>("button"))?.focus();
      } else if (document.activeElement === document.body || menu.contains(document.activeElement)) buttonRef.current?.focus();
    };
    menu.addEventListener("toggle", onToggle);
    return () => menu.removeEventListener("toggle", onToggle);
  }, []);
  const close = (): void => {
    try { if (menuRef.current?.matches(":popover-open")) menuRef.current.hidePopover(); } catch { /* No popover support (tests). */ }
  };
  const keyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key === "Tab") { close(); return; }
    const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button")];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "ArrowDown" ? buttons[(index + 1) % buttons.length] : event.key === "ArrowUp" ? buttons[(index - 1 + buttons.length) % buttons.length]
      : event.key === "Home" ? buttons[0] : event.key === "End" ? buttons.at(-1) : undefined;
    if (!next) return;
    event.preventDefault();
    next.focus();
  };
  return <>
    <button ref={buttonRef} className="icon-button tab-overflow" type="button" title={label} aria-label={label} aria-haspopup="menu"
      aria-expanded={open} aria-controls={menuId} popoverTarget={menuId}><ChevronDownIcon /></button>
    <div ref={menuRef} id={menuId} className="sftp-menu tab-overflow-menu" popover="auto" role="menu" aria-label={label} onKeyDown={keyDown}>
      {tabs.map((tab) => <button key={tab.id} type="button" role="menuitem" aria-current={tab.selected ? "page" : undefined}
        onClick={() => { close(); onSelect(tab.id); }}>
        <span className="led" data-state={tab.signal === "starting" ? "connecting" : tab.signal ?? undefined} data-dark={tab.signal ? undefined : "true"} aria-hidden="true" />
        <span className="sftp-menu-label">{tab.name}</span>
        {tab.production && <span className="prod-chip">{productionLabel}</span>}
      </button>)}
    </div>
  </>;
}
