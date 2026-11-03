import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type ReactElement } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { copy } from "../i18n";
import { readRecentCommands, rememberCommand, searchCommands, type PaletteAction } from "./registry";
import type { CommandAction } from "./contracts";
import {
  BroadcastIcon, CloseIcon, DownloadIcon, FolderIcon, LockIcon, PlusIcon, SearchIcon, ServerIcon, SettingsIcon, ShellIcon,
  SidebarIcon, SnippetIcon, SplitDownIcon, SplitRightIcon, ThemeIcon, TunnelIcon,
} from "../ui/icons";

type Labels = typeof copy.en | typeof copy["zh-CN"];
type Props = {
  open: boolean;
  labels: Labels;
  actions: readonly PaletteAction[];
  onOpenChange: (open: boolean) => void;
  onExecute: (action: PaletteAction["action"]) => void;
};
type Section = keyof Labels["paletteSections"];
const sectionOrder: readonly Exclude<Section, "recent">[] = ["hosts", "snippets", "shells", "workspace", "panels", "appearance"];
const RECENT_SHOWN = 5;

function sectionOf(action: CommandAction): Exclude<Section, "recent"> {
  switch (action.id) {
    case "connect_host": return "hosts";
    case "run_snippet": return "snippets";
    case "open_shell": return "shells";
    case "workspace": return "workspace";
    case "set_theme": return "appearance";
    case "toggle_panel": case "open_settings": return "panels";
  }
}

function iconOf(action: CommandAction): ReactElement {
  switch (action.id) {
    case "connect_host": return <ServerIcon size={14} />;
    case "run_snippet": return <SnippetIcon size={14} />;
    case "open_shell": return <ShellIcon size={14} />;
    case "set_theme": return <ThemeIcon size={14} />;
    case "open_settings": return <SettingsIcon size={14} />;
    case "toggle_panel": return action.panel === "forwarding" ? <TunnelIcon size={14} /> : <SnippetIcon size={14} />;
    case "workspace": switch (action.command) {
      case "new_terminal": return <PlusIcon size={14} />;
      case "close_terminal": return <CloseIcon size={14} />;
      case "split_right": return <SplitRightIcon size={14} />;
      case "split_down": return <SplitDownIcon size={14} />;
      case "toggle_broadcast": return <BroadcastIcon size={14} />;
      case "toggle_sidebar": return <SidebarIcon size={14} />;
      case "focus_files": return <FolderIcon size={14} />;
      case "focus_transfers": return <DownloadIcon size={14} />;
      case "lock_vault": return <LockIcon size={14} />;
    }
  }
}

/**
 * A Panel popover over the workspace. Focus stays in the search field
 * (combobox pattern); the active option is announced through
 * aria-activedescendant and kept in view while arrowing.
 */
export function CommandPalette({ open, labels, actions, onOpenChange, onExecute }: Props): ReactElement {
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const chosenRef = useRef(false);
  // A chosen action decides where focus goes; only a dismissed palette restores it.
  const handOffRef = useRef(false);
  const listId = useId();
  const recentKeys = useMemo(() => open ? readRecentCommands() : [], [open]);
  const sections = useMemo(() => {
    if (query.trim()) return [{ id: "results", label: null, items: searchCommands(actions, query) }];
    const recent = recentKeys.flatMap((key) => actions.filter((action) => action.key === key)).slice(0, RECENT_SHOWN);
    const rest = actions.filter((action) => !recent.includes(action));
    return [
      { id: "recent", label: labels.paletteSections.recent, items: recent },
      ...sectionOrder.map((section) => ({ id: section, label: labels.paletteSections[section], items: rest.filter((action) => sectionOf(action.action) === section) })),
    ].filter((section) => section.items.length > 0);
  }, [actions, labels, query, recentKeys]);
  const matches = useMemo(() => sections.flatMap((section) => section.items), [sections]);
  const optionId = (index: number): string => `${listId}-option-${index}`;

  useEffect(() => {
    if (!open) { chosenRef.current = false; setQuery(""); setActiveIndex(0); }
    else handOffRef.current = false;
  }, [open]);
  useEffect(() => setActiveIndex(0), [query]);
  useEffect(() => setActiveIndex((index) => Math.min(index, Math.max(0, matches.length - 1))), [matches.length]);
  useEffect(() => {
    if (open) document.getElementById(optionId(activeIndex))?.scrollIntoView?.({ block: "nearest" });
    // optionId only depends on the stable list ID.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, activeIndex, matches]);

  const choose = (index: number): void => {
    const selected = matches[index];
    if (!open || chosenRef.current || !selected) return;
    chosenRef.current = true;
    handOffRef.current = true;
    rememberCommand(selected.key);
    onOpenChange(false);
    onExecute(selected.action);
  };
  const onSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>): void => {
    // IME candidate confirmation/navigation belongs to the input method. Some
    // browsers report keyCode 229 while isComposing is already false.
    if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
    const last = matches.length - 1;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((index) => matches.length === 0 ? 0 : (index + direction + matches.length) % matches.length);
    } else if ((event.key === "PageDown" || event.key === "PageUp") && last >= 0) {
      event.preventDefault();
      setActiveIndex((index) => Math.max(0, Math.min(last, index + (event.key === "PageDown" ? 8 : -8))));
    } else if ((event.key === "Home" || event.key === "End") && event.ctrlKey && last >= 0) {
      event.preventDefault();
      setActiveIndex(event.key === "Home" ? 0 : last);
    } else if (event.key === "Enter") {
      event.preventDefault();
      choose(activeIndex);
    }
  };

  let position = -1;
  return <Dialog.Root open={open} onOpenChange={onOpenChange}>
    <Dialog.Portal>
      <Dialog.Overlay className="command-palette-overlay" />
      <Dialog.Content className="command-palette" onOpenAutoFocus={(event) => { event.preventDefault(); inputRef.current?.focus(); }}
        onCloseAutoFocus={(event) => { if (handOffRef.current) { handOffRef.current = false; event.preventDefault(); } }}
        onEscapeKeyDown={(event) => { if (event.isComposing || event.keyCode === 229) event.preventDefault(); }}>
        <Dialog.Title className="sr-only">{labels.commandPalette}</Dialog.Title>
        <Dialog.Description className="sr-only">{labels.searchCommands}</Dialog.Description>
        <label className="command-search"><SearchIcon size={14} /><input ref={inputRef} type="search" autoComplete="off" spellCheck={false}
          role="combobox" aria-expanded="true" aria-autocomplete="list" aria-controls={listId} aria-label={labels.searchCommands}
          aria-activedescendant={matches.length > 0 ? optionId(activeIndex) : undefined} placeholder={labels.searchCommands}
          value={query} onChange={(event) => setQuery(event.currentTarget.value)} onKeyDown={onSearchKeyDown} /></label>
        <div className="command-results" id={listId} role="listbox" aria-label={labels.commandPalette}>
          {sections.map((section) => <div key={section.id} role="group" aria-labelledby={section.label ? `${listId}-${section.id}` : undefined}>
            {section.label && <div className="command-section" id={`${listId}-${section.id}`} role="presentation">{section.label}</div>}
            {section.items.map((candidate) => {
              const index = ++position;
              return <div key={candidate.key} id={optionId(index)} role="option" aria-selected={index === activeIndex} className="command-result"
                onMouseDown={(event) => event.preventDefault()} onMouseMove={() => { if (index !== activeIndex) setActiveIndex(index); }} onClick={() => choose(index)}>
                <span className="command-icon" aria-hidden="true">{iconOf(candidate.action)}</span>
                <span className="command-label">{candidate.label}</span>
                {candidate.production && <span className="prod-chip">{labels.production}</span>}
                {candidate.detail && <span className={"command-detail" + (candidate.detailMono ? " mono" : "")}>{candidate.detail}</span>}
                {candidate.shortcut && <kbd>{candidate.shortcut}</kbd>}
              </div>;
            })}
          </div>)}
          {matches.length === 0 && <p className="command-empty" role="status">{labels.noCommandMatches}</p>}
        </div>
        <p className="command-hint" aria-hidden="true">
          <span><kbd>↑</kbd><kbd>↓</kbd>{labels.paletteMove}</span>
          <span><kbd>Enter</kbd>{labels.paletteRun}</span>
          <span><kbd>Esc</kbd>{labels.paletteClose}</span>
        </p>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
