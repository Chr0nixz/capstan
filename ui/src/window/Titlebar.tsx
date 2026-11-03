import { useEffect, useRef, useState, type ReactElement } from "react";
import { invokeTauri, isTauriAvailable } from "../terminal/bridge";
import { createSnapController, MAXIMIZE_BUTTON_ID } from "./snap";
import type { WindowCommand } from "../terminal/contracts";
import { CloseGlyph, MaximizeGlyph, MinimizeGlyph } from "../ui/icons";

type Labels = { title: string; localShell: string; windowControls: string; minimize: string; maximize: string;
  close: string; snapUnavailable: string };

/** `mono` sets the context in the data face; it is on for `user@host`, off for shell names. */
export function Titlebar({ labels, context = labels.localShell, badge, mono = false }: { labels: Labels; context?: string; badge?: string; mono?: boolean }): ReactElement {
  const maximizeRef = useRef<HTMLButtonElement>(null);
  const [snapFailed, setSnapFailed] = useState(false);
  useEffect(() => {
    if (!maximizeRef.current || !isTauriAvailable()) return;
    const snap = createSnapController(maximizeRef.current, () => setSnapFailed(true));
    return () => { void snap.dispose().catch(() => undefined); };
  }, []);
  const command = (operation: WindowCommand): void => { void invokeTauri(operation).catch(() => undefined); };
  return <header className="titlebar">
    <div className="titlebar-drag" data-tauri-drag-region>
      <h1 className="brand" data-tauri-drag-region>{labels.title}</h1>
      <span className={"context" + (mono ? " is-mono" : "")} data-tauri-drag-region>{badge && <span className="context-badge" data-tauri-drag-region>{badge}</span>}{context}</span>
      {snapFailed && <span className="titlebar-warning" role="status">{labels.snapUnavailable}</span>}
    </div>
    <nav className="window-controls" aria-label={labels.windowControls}>
      <button type="button" title={labels.minimize} aria-label={labels.minimize} onClick={() => command("minimize_window")}>
        <MinimizeGlyph />
      </button>
      <button ref={maximizeRef} id={MAXIMIZE_BUTTON_ID} type="button" title={labels.maximize} aria-label={labels.maximize} onClick={() => command("toggle_maximize_window")}>
        <MaximizeGlyph />
      </button>
      <button className="close-window" type="button" title={labels.close} aria-label={labels.close} onClick={() => command("close_window")}>
        <CloseGlyph />
      </button>
    </nav>
  </header>;
}
