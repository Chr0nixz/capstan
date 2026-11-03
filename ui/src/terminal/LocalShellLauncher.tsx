import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactElement } from "react";
import { copy } from "../i18n";
import { invokeTauri } from "./bridge";
import type { LocalShellProfile, LocalTerminalOptions } from "./contracts";
import { ChevronDownIcon, FolderIcon, ShellIcon } from "../ui/icons";

type Labels = typeof copy.en | typeof copy["zh-CN"];

/**
 * The chevron beside +: a menu of detected shells where one click or Enter
 * opens a terminal. Picking a starting directory is the last item.
 */
export function LocalShellLauncher({ labels, onLaunch }: {
  labels: Labels;
  onLaunch: (options: LocalTerminalOptions, label?: string) => void;
}): ReactElement {
  const details = useRef<HTMLDetailsElement>(null);
  const summary = useRef<HTMLElement>(null);
  const panel = useRef<HTMLElement>(null);
  const [open, setOpen] = useState(false);
  const [custom, setCustom] = useState(false);
  const [profiles, setProfiles] = useState<LocalShellProfile[]>([]);
  const [shellId, setShellId] = useState("");
  const [directory, setDirectory] = useState("");
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!open) { setCustom(false); return; }
    let disposed = false;
    setLoading(true); setFailed(false);
    void invokeTauri("list_local_shells").then((found) => {
      if (disposed) return;
      setProfiles(found);
      setShellId((current) => found.some((profile) => profile.id === current) ? current : "");
    }).catch(() => { if (!disposed) setFailed(true); })
      .finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [open, retry]);
  // The panel is fixed (the tab strip clips overflow); pin it under its chevron.
  useLayoutEffect(() => {
    if (!open) return;
    const place = (): void => {
      const anchor = summary.current?.getBoundingClientRect(), element = panel.current;
      if (!anchor || !element) return;
      element.style.top = `${Math.round(anchor.bottom + 6)}px`;
      element.style.left = `${Math.round(Math.max(8, Math.min(anchor.left - 8, window.innerWidth - element.offsetWidth - 8)))}px`;
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [open, custom]);
  // Keyboard users land on the first shell; the form lands on its first field.
  useLayoutEffect(() => {
    if (!open) return;
    panel.current?.querySelector<HTMLElement>(custom ? "select" : "[role='menuitem']")?.focus();
  }, [open, custom]);
  const close = (): void => {
    if (details.current) details.current.open = false;
    setOpen(false);
  };
  const launch = (options: LocalTerminalOptions, label?: string): void => {
    close();
    onLaunch(options, label);
  };
  const menuKeyDown = (event: KeyboardEvent<HTMLElement>): void => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const items = [...event.currentTarget.querySelectorAll<HTMLElement>("[role='menuitem']:not(:disabled)")];
    const index = items.indexOf(document.activeElement as HTMLElement);
    const next = event.key === "Home" ? items[0] : event.key === "End" ? items.at(-1)
      : items[(index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length];
    next?.focus();
  };
  const selected = profiles.find((profile) => profile.id === shellId);
  return <details ref={details} className="local-launcher" onToggle={(event) => setOpen(event.currentTarget.open)} onKeyDown={(event) => {
    if (event.key === "Escape") { event.preventDefault(); close(); summary.current?.focus(); }
  }}>
    <summary ref={summary} title={labels.chooseLocalShell} aria-label={labels.chooseLocalShell} aria-haspopup="menu">
      <ChevronDownIcon size={14} />
    </summary>
    {open && <section ref={panel} className="local-launcher-panel" data-mode={custom ? "custom" : "menu"} aria-label={labels.chooseLocalShell}>
      {!custom ? <div className="launcher-menu" role="menu" aria-label={labels.chooseLocalShell} onKeyDown={menuKeyDown}>
        <button type="button" role="menuitem" onClick={() => launch({})}>
          <ShellIcon size={14} /><span className="launcher-label">{labels.defaultLocalShell}</span><kbd>Ctrl+Shift+T</kbd>
        </button>
        {profiles.map((profile) => <button key={profile.id} type="button" role="menuitem" onClick={() => launch({ shell_id: profile.id }, profile.name)}>
          <ShellIcon size={14} /><span className="launcher-label">{profile.name}</span>
        </button>)}
        {loading && <p className="launcher-note" role="status">{labels.loadingShells}</p>}
        {failed && <p className="launcher-error" role="alert">{labels.shellDiscoveryFailed} <button type="button" onClick={() => setRetry((value) => value + 1)}>{labels.retryShells}</button></p>}
        <div className="launcher-separator" role="separator" />
        <button type="button" role="menuitem" onClick={() => setCustom(true)}><FolderIcon size={14} /><span className="launcher-label">{labels.customLocalShell}</span></button>
      </div>
        : <form onSubmit={(event) => {
          event.preventDefault();
          const options: LocalTerminalOptions = {};
          if (shellId) options.shell_id = shellId;
          if (directory.trim()) options.working_directory = directory.trim();
          launch(options, selected?.name);
        }}>
          <label>{labels.localShellProfile}<select value={shellId} onChange={(event) => { setShellId(event.currentTarget.value); setDirectory(""); }}>
            <option value="">{labels.defaultLocalShell}</option>
            {profiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}
          </select></label>
          <label>{labels.localWorkingDirectory}<input value={directory} onChange={(event) => setDirectory(event.currentTarget.value)} maxLength={4096} placeholder={selected?.is_wsl ? "/home" : labels.defaultDirectory} /></label>
          <p className="quiet-copy">{selected?.is_wsl ? labels.wslDirectoryHint : labels.localDirectoryHint}</p>
          <div className="launcher-actions">
            <button type="submit" className="primary-action" disabled={loading}>{labels.openLocalShell}</button>
            <button type="button" className="ghost-button" onClick={() => setCustom(false)}>{labels.backToShells}</button>
          </div>
        </form>}
    </section>}
  </details>;
}
