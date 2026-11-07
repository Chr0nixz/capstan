import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type ReactElement } from "react";
import { paneStatus, useWorkspace } from "../workspace";
import { routeInput, type PaneId } from "../panes/contracts";
import { findPane } from "../panes/tree";
import { paneInputs, type ConfirmBroadcast, type InputDeliveryResult } from "../panes/input";
import { RoutedInputQueue } from "../panes/inputQueue";
import { invokeTauri, isTauriAvailable } from "./bridge";
import { createRecoveringController } from "../connections/controller";
import { useTunnelTargets } from "../forwarding/targets";
import type { ConnectionSnapshot } from "../connections/contracts";
import type { TabId, TerminalLaunch, TerminalTheme } from "./contracts";
import { createTerminalSession, type TerminalSession } from "./session";
import { cwdHints } from "../sftp/cwdHints";
import type { TerminalSearchOptions } from "./search";
import { activateTerminalLink } from "./security";
import { ArrowDownIcon, ArrowUpIcon, CheckIcon, CloseIcon, RetryIcon } from "../ui/icons";
import { flushSync } from "react-dom";
import { saveDesktopPreference, useDesktopPreferences, useTerminalThemes } from "../themes/store";

export type TerminalPaneLabels = {
  localShell: string;
  unavailable: string;
  startFailed: string;
  inputOverflow: string;
  inputFailed: string;
  outputInvalid: string;
  operationFailed: string;
  hostKeyChanged: string;
  authenticationFailed: string;
  connectionFailed: string;
  connectionTimedOut: string;
  shellRejected: string;
  operationCanceled: string;
  privateKeyInvalid: string;
  credentialFileInvalid: string;
  agentUnavailable: string;
  agentIdentityUnavailable: string;
  vaultLocked: string;
  invalidInput: string;
  workingDirectoryInvalid: string;
  exited: (code: number | null) => string;
  searchTerminal: string;
  previousMatch: string;
  nextMatch: string;
  caseSensitiveSearch: string;
  regexSearch: string;
  closeSearch: string;
  searchCount: (index: number, count: number) => string;
  searchNoMatches: string;
  searchInvalidRegex: string;
  confirmMultilinePaste: (lineCount: number) => string;
  confirmOsc52Write: (characterCount: number) => string;
  osc52ReadBlocked: string;
  osc52WriteFailed: string;
  confirmExternalLink: (protocol: string, url: string) => string;
  externalLinkFailed: string;
  sessionEnded: string;
  sessionFailed: string;
  reconnect: string;
  reconnecting: string;
  reconnected: string;
  cancelReconnect: string;
  restartShell: string;
  pressEnter: string;
  pressEnterProduction: string;
  production: string;
  closeTerminal: string;
  terminalMenu: string;
  terminalCopy: string;
  terminalPaste: string;
  terminalClear: string;
  copyOnSelect: string;
  terminalClipboardError: string;
  desktopPreferencesError: string;
  dismiss: string;
};
type TerminalPaneProps = {
  tabId: TabId; launch: TerminalLaunch; visible: boolean; labels: TerminalPaneLabels;
  paneId?: PaneId;
  focused?: boolean;
  confirmBroadcast?: ConfirmBroadcast;
  onInputResult?: (result: InputDeliveryResult) => void;
  /** Bumped by tab strip actions; the visible pane then takes keyboard focus. */
  focusRequest?: number;
  /** Accessible name of the terminal surface, e.g. the SSH host. */
  label?: string;
  /** Starts a new session in this tab once the current one has ended. */
  onRestart?: () => void;
  onClose?: () => void;
  /** Production sessions never restart from a stray Enter; it only selects Reconnect. */
  production?: boolean;
  /** Optional command-palette theme override, applied without restarting the shell. */
  themeOverride?: TerminalTheme;
};

export function TerminalPane({ tabId, paneId, launch, visible, focused = true, labels, focusRequest, label, onRestart, onClose, production = false, confirmBroadcast, onInputResult, themeOverride }: TerminalPaneProps): ReactElement {
  const containerRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuAnchor = useRef({ x: 0, y: 0 });
  const [canCopy, setCanCopy] = useState(false);
  const [menuError, setMenuError] = useState<string | null>(null);
  const { preferences, ready: preferencesReady, saving: preferencesSaving } = useDesktopPreferences();
  const themes = useTerminalThemes((state) => state.themes);
  const theme = themeOverride ?? (launch.kind === "ssh" ? launch.theme : "graphite");
  const themeBackground = themes.find(({ id }) => id === theme)?.background;
  const restartRef = useRef<HTMLButtonElement>(null);
  const sessionRef = useRef<TerminalSession | null>(null);
  const controllerRef = useRef<ReturnType<typeof createRecoveringController> | null>(null);
  const [connection, setConnection] = useState<ConnectionSnapshot | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [regex, setRegex] = useState(false);
  const [matchIndex, setMatchIndex] = useState(-1);
  const [matchCount, setMatchCount] = useState(0);
  const [invalidRegex, setInvalidRegex] = useState(false);
  const status = useWorkspace((state) => {
    const tab = state.tabs.find((candidate) => candidate.id === tabId);
    return tab ? paneStatus(tab, paneId ?? tab.focusedPaneId) : null;
  });
  const ended = status === "ended" || status === "failed";
  const recovering = connection && ["connecting", "authenticating", "reconnecting"].includes(connection.state);
  const restart = (): void => {
    if (controllerRef.current?.canReconnect) void controllerRef.current.reconnect().catch(() => {
      sessionRef.current?.writeLocal(new TextEncoder().encode(`\r\n${labels.operationFailed}\r\n`));
    });
    else onRestart?.();
  };
  const labelsRef = useRef(labels);
  const visibleRef = useRef(visible);
  const inputCallbacksRef = useRef({ confirmBroadcast, onInputResult });
  inputCallbacksRef.current = { confirmBroadcast, onInputResult };
  labelsRef.current = labels;
  visibleRef.current = visible;

  const runSearch = useCallback((
    query: string,
    options: TerminalSearchOptions,
    direction: 1 | -1,
    currentIndex: number,
  ): void => {
    const result = sessionRef.current?.search(query, options, direction, currentIndex);
    if (!result) return;
    setMatchIndex(result.index);
    setMatchCount(result.count);
    setInvalidRegex(result.invalidExpression);
  }, []);
  const searchOptions: TerminalSearchOptions = { caseSensitive, regex };
  const moveMatch = (direction: 1 | -1): void => {
    runSearch(searchQuery, searchOptions, direction, matchIndex);
  };
  const closeSearch = (): void => {
    sessionRef.current?.clearSearch();
    setSearchOpen(false);
    setSearchQuery("");
    setMatchIndex(-1);
    setMatchCount(0);
    setInvalidRegex(false);
    requestAnimationFrame(() => sessionRef.current?.focus());
  };
  const closeMenu = (): void => {
    const menu = menuRef.current;
    if (typeof menu?.hidePopover === "function" && menu.matches(":popover-open")) menu.hidePopover();
  };
  const openMenuAt = (x: number, y: number): void => {
    const menu = menuRef.current;
    if (!menu?.showPopover || !sessionRef.current) return;
    closeMenu();
    menuAnchor.current = { x, y };
    flushSync(() => { setCanCopy(sessionRef.current?.hasSelection() ?? false); setMenuError(null); });
    menu.showPopover();
  };
  const menuKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key === "Escape" || event.key === "Tab") {
      closeMenu();
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        sessionRef.current?.focus();
      }
      return;
    }
    const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "ArrowDown" ? items[(index + 1) % items.length] : event.key === "ArrowUp" ? items[(index - 1 + items.length) % items.length]
      : event.key === "Home" ? items[0] : event.key === "End" ? items.at(-1) : undefined;
    if (!next) return;
    event.preventDefault();
    next.focus();
  };
  const runMenuAction = (action: (session: TerminalSession) => void | Promise<void>, error: string): void => {
    const session = sessionRef.current;
    closeMenu();
    session?.focus();
    if (session) void Promise.resolve(action(session)).catch(() => setMenuError(error));
  };
  const onPaneKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if ((event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) && containerRef.current?.contains(event.target as Node)) {
      event.preventDefault();
      event.stopPropagation();
      const rect = (event.target as HTMLElement).getBoundingClientRect();
      openMenuAt(rect.left + 12, rect.bottom);
      return;
    }
    if (searchOpen && event.key === "Escape") {
      event.preventDefault();
      closeSearch();
      return;
    }
    if (event.ctrlKey && event.shiftKey && event.key.toLocaleLowerCase() === "f") {
      event.preventDefault();
      setSearchOpen(true);
      return;
    }
    // Like Windows Terminal: once the session is over, Enter starts it again.
    if (ended && !recovering && onRestart && event.key === "Enter" && !event.ctrlKey && !event.altKey && !event.shiftKey && !searchOpen
      && (event.target as HTMLElement).closest(".terminal-host")) {
      event.preventDefault();
      if (production) restartRef.current?.focus(); else restart();
    }
  };

  useEffect(() => {
    if (searchOpen) searchInputRef.current?.focus();
  }, [searchOpen]);

  useEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const onToggle = (event: Event): void => {
      if ((event as ToggleEvent).newState === "open") {
        const { x, y } = menuAnchor.current;
        menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - menu.offsetWidth - 4))}px`;
        menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - menu.offsetHeight - 4))}px`;
        menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
      } else if (document.activeElement === document.body || menu.contains(document.activeElement)) {
        sessionRef.current?.focus();
      }
    };
    menu.addEventListener("toggle", onToggle);
    return () => menu.removeEventListener("toggle", onToggle);
  }, []);

  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    let controller: ReturnType<typeof createRecoveringController> | undefined;
    let unregisterInput: (() => void) | undefined;
    let disposed = false;
    let cwdPublisher: ReturnType<typeof cwdHints.open> | undefined;
    let size = { columns: 80, rows: 24 };
    let inputVersion = 0;
    let hasStarted = false;
    setConnection(null);
    const captureInput = (bytes: Uint8Array) => {
      const workspace = useWorkspace.getState(), tab = workspace.tabs.find((tab) => tab.id === tabId);
      const broadcast = !!paneId && !!tab?.broadcastEnabled && workspace.activeTab === tabId && tab.focusedPaneId === paneId;
      const route = routeInput(paneId ?? tab?.focusedPaneId ?? ("standalone" as PaneId), tab?.broadcastPaneIds ?? [], broadcast,
        broadcast ? new TextDecoder().decode(bytes) : "", tab?.broadcastArmedPaneIds ?? []);
      return { send: paneInputs.prepare(route), broadcast };
    };
    const inputQueue = paneId ? new RoutedInputQueue(captureInput) : undefined;
    const localMessage = (message: string): void => session.writeLocal(new TextEncoder().encode(`\r\n${message}\r\n`));
    const session = createTerminalSession(element, visibleRef.current, {
      theme,
      themes,
      inputQueue,
      sendInput: async (bytes) => {
        if (!paneId) return controller?.sendInput(bytes) ?? Promise.reject(new Error("session_closed"));
        const captured = inputQueue?.route() ?? captureInput(bytes);
        const isCurrent = (): boolean => {
          const state = useWorkspace.getState(), current = state.tabs.find((tab) => tab.id === tabId);
          return !disposed && !!current && findPane(current.root, paneId)?.launch === launch
            && (!captured.broadcast || state.activeTab === tabId);
        };
        if (!isCurrent()) return;
        // Only user-originated input enters this queue and can join a broadcast.
        const result = await captured.send(bytes, (targets, reason) => inputCallbacksRef.current.confirmBroadcast?.(targets, reason) ?? Promise.resolve(false), isCurrent);
        if (!disposed) inputCallbacksRef.current.onInputResult?.(result);
      },
      sendProtocolInput: async (bytes) => {
        if (!paneId) return controller?.sendInput(bytes) ?? Promise.reject(new Error("session_closed"));
        const isCurrent = (): boolean => {
          const tab = useWorkspace.getState().tabs.find((tab) => tab.id === tabId);
          return !disposed && !!tab && findPane(tab.root, paneId)?.launch === launch;
        };
        if (!isCurrent()) return;
        // Replies use the source's existing bounded destination FIFO, without
        // waiting behind a user broadcast's confirmation or consulting selection.
        const result = await paneInputs.dispatch(routeInput(paneId, [], false, ""), bytes, async () => false, isCurrent);
        if (!disposed) inputCallbacksRef.current.onInputResult?.(result);
      },
      ackOutput: (bytes) => controller?.acknowledge(bytes),
      onResize: (columns, rows) => { size = { columns, rows }; controller?.resize(columns, rows); },
      // Broadcast confirmation gates the complete encoded input (including
      // bracketed paste) once, before any target receives bytes.
      confirmMultilinePaste: (lineCount) => !!paneId && !!useWorkspace.getState().tabs.find((tab) => tab.id === tabId)?.broadcastEnabled
        || window.confirm(labelsRef.current.confirmMultilinePaste(lineCount)),
      copyOnSelect: () => useDesktopPreferences.getState().preferences.copyOnSelect,
      readClipboard: () => navigator.clipboard.readText(),
      writeSelectionClipboard: async (text) => navigator.clipboard.writeText(text),
      onSelectionCopyFailed: () => setMenuError(labelsRef.current.terminalClipboardError),
      confirmOsc52Write: (characterCount) => window.confirm(labelsRef.current.confirmOsc52Write(characterCount)),
      writeOsc52Clipboard: (text) => navigator.clipboard.writeText(text),
      onOsc52ReadBlocked: () => localMessage(labelsRef.current.osc52ReadBlocked),
      onOsc52WriteFailed: () => localMessage(labelsRef.current.osc52WriteFailed),
      onRemoteCwd: (uri) => cwdPublisher?.publish(uri),
      onActivateLink: (uri) => {
        activateTerminalLink(
          uri,
          (url) => window.confirm(labelsRef.current.confirmExternalLink(url.protocol, url.href)),
          (url) => {
            if (isTauriAvailable()) {
              void invokeTauri("open_external_url", {
                url: url.href,
                confirmedNonHttp: url.protocol !== "http:" && url.protocol !== "https:",
              }).catch(() => localMessage(labelsRef.current.externalLinkFailed));
            } else {
              window.open(url.href, "_blank", "noopener,noreferrer");
            }
          },
        );
      },
      onInputOverflow: () => localMessage(labelsRef.current.inputOverflow),
      onInputError: () => {
        // Keys typed after the process ended have no recipient. Its status
        // message follows once the remaining output has been delivered.
        if (controller?.ended) return;
        localMessage(labelsRef.current.inputFailed);
        if (paneId) useWorkspace.getState().failTab(tabId, paneId, launch);
        void controller?.dispose().catch(() => undefined);
      },
    });
    sessionRef.current = session;
    if (isTauriAvailable()) {
      controller = createRecoveringController(launch, {
        connection: (snapshot) => { if (!disposed) setConnection(snapshot); },
        suspended: () => { inputVersion += 1; unregisterInput?.(); session.stopInput(); cwdPublisher?.close(); },
        replacing: async () => {
          await session.settleOutput();
          if (!disposed) session.writeLocal(new TextEncoder().encode("\x18\x1b[?1049l\x1b[?2004l\x1b[0m"));
        },
        started: (started) => {
          if (started.kind === "ssh") useTunnelTargets.getState().register(started.hostId, started.connectionId);
          if (hasStarted) localMessage(labelsRef.current.reconnected);
          hasStarted = true;
          session.resumeInput();
          if (started.kind === "ssh") { cwdPublisher?.close(); cwdPublisher = cwdHints.open(started.id); }
          if (paneId) {
            unregisterInput?.();
            unregisterInput = registerInput();
          }
        },
        output: (bytes) => session.write(bytes),
        status: (snapshot) => {
          useWorkspace.getState().updateSession(tabId, snapshot, paneId, launch);
          if (snapshot.state !== "running") {
            cwdPublisher?.close();
            session.stopInput();
            localMessage(snapshot.state === "exited" ? labelsRef.current.exited(snapshot.exit_code) : labelsRef.current.operationFailed);
          }
        },
        failure: (failure) => {
          cwdPublisher?.close();
          session.stopInput();
          useWorkspace.getState().failTab(tabId, paneId, launch);
          localMessage(labelsRef.current[failure]);
        },
      });
      controller.resize(size.columns, size.rows);
      controllerRef.current = controller;
      const registerInput = () => {
        const identity = ++inputVersion;
        return paneInputs.register(paneId!, async (bytes) => {
        try {
          if (!controller) throw { code: "session_closed" };
          await controller.sendInput(bytes);
        } catch (cause) {
          // This destination's input FIFO closes after a failed/uncertain
          // write. Expose a restart action instead of leaving a live-looking
          // shell whose input can no longer be accepted.
          if (!disposed && identity === inputVersion && !controller?.ended) {
            session.stopInput();
            useWorkspace.getState().failTab(tabId, paneId, launch);
            localMessage(labelsRef.current.inputFailed);
            void controller?.dispose().catch(() => undefined);
          }
          throw cause;
        }
        });
      };
      if (paneId) unregisterInput = registerInput();
    } else {
      session.stopInput();
      useWorkspace.getState().failTab(tabId, paneId, launch);
      localMessage(labelsRef.current.unavailable);
    }
    return () => {
      disposed = true;
      unregisterInput?.();
      cwdPublisher?.close();
      void controller?.dispose().catch(() => undefined);
      session.dispose();
      sessionRef.current = null;
      controllerRef.current = null;
    };
  }, [launch, tabId, paneId]);

  useEffect(() => { sessionRef.current?.setVisible(visible); if (!visible) closeMenu(); }, [visible, tabId]);
  useEffect(() => { sessionRef.current?.setTheme(theme, themes); }, [theme, themes]);
  // Focusing a checkbox/header in a different pane must not steal focus back
  // into xterm. Tab activation and explicit terminal actions request it here.
  useEffect(() => { if (visible && focused) sessionRef.current?.focus(); }, [visible, focusRequest, tabId, launch]);
  return <div className="terminal-pane-root" onKeyDownCapture={onPaneKeyDown}>
    {searchOpen && <form className="terminal-search-toolbar" onSubmit={(event) => { event.preventDefault(); moveMatch(1); }}>
      <input
        ref={searchInputRef}
        aria-label={labels.searchTerminal}
        placeholder={labels.searchTerminal}
        value={searchQuery}
        onChange={(event) => {
          const query = event.currentTarget.value;
          setSearchQuery(query);
          runSearch(query, searchOptions, 1, -1);
        }}
      />
      <button className="icon-button" type="button" aria-label={labels.previousMatch} title={labels.previousMatch} onClick={() => moveMatch(-1)}><ArrowUpIcon /></button>
      <button className="icon-button" type="submit" aria-label={labels.nextMatch} title={labels.nextMatch}><ArrowDownIcon /></button>
      <label className="search-toggle" title={labels.caseSensitiveSearch}><input type="checkbox" aria-label={labels.caseSensitiveSearch} checked={caseSensitive} onChange={(event) => {
        const value = event.currentTarget.checked;
        setCaseSensitive(value);
        runSearch(searchQuery, { caseSensitive: value, regex }, 1, -1);
      }} />Aa</label>
      <label className="search-toggle" title={labels.regexSearch}><input type="checkbox" aria-label={labels.regexSearch} checked={regex} onChange={(event) => {
        const value = event.currentTarget.checked;
        setRegex(value);
        runSearch(searchQuery, { caseSensitive, regex: value }, 1, -1);
      }} />.*</label>
      <span className="search-count" aria-live="polite">{invalidRegex ? labels.searchInvalidRegex : !searchQuery ? "" : matchCount === 0 ? labels.searchNoMatches : labels.searchCount(matchIndex + 1, matchCount)}</span>
      <button className="icon-button" type="button" aria-label={labels.closeSearch} title={labels.closeSearch} onClick={closeSearch}><CloseIcon /></button>
    </form>}
    <div ref={containerRef} className="terminal-host" data-theme={theme} aria-label={label ?? labels.localShell} aria-keyshortcuts="Shift+F10 ContextMenu"
      aria-hidden={!visible} style={{ display: visible ? "block" : "none", backgroundColor: themeBackground }}
      onMouseDownCapture={(event) => {
        if (event.button === 2 && event.shiftKey) { event.preventDefault(); event.stopPropagation(); }
      }}
      onContextMenuCapture={(event) => {
        event.preventDefault();
        event.stopPropagation();
        if (event.shiftKey || !sessionRef.current?.mouseReporting()) openMenuAt(event.clientX, event.clientY);
      }} />
    <div ref={menuRef} className="sftp-menu terminal-menu" popover="auto" role="menu" aria-label={labels.terminalMenu} data-theme={theme} onKeyDown={menuKeyDown}>
      <button type="button" role="menuitem" tabIndex={-1} disabled={!canCopy} onClick={() => runMenuAction((session) => session.copySelection(), labels.terminalClipboardError)}>{labels.terminalCopy}</button>
      <button type="button" role="menuitem" tabIndex={-1} disabled={ended || !!recovering} onClick={() => runMenuAction((session) => session.pasteFromClipboard(), labels.terminalClipboardError)}>{labels.terminalPaste}</button>
      <button type="button" role="menuitem" tabIndex={-1} onClick={() => runMenuAction((session) => session.clearScreen(), labels.operationFailed)}>{labels.terminalClear}</button>
      <div className="sftp-menu-separator" role="separator" />
      <button type="button" role="menuitemcheckbox" tabIndex={-1} aria-checked={preferences.copyOnSelect} disabled={!preferencesReady || preferencesSaving}
        onClick={() => runMenuAction(() => saveDesktopPreference("copyOnSelect", !preferences.copyOnSelect), labels.desktopPreferencesError)}>
        <span className="sftp-menu-label">{labels.copyOnSelect}</span>{preferences.copyOnSelect && <CheckIcon size={14} className="sftp-menu-check" />}
      </button>
    </div>
    {menuError && <div className="pane-input-error" role="alert"><span>{menuError}</span><button className="icon-button" type="button" aria-label={labels.dismiss} onClick={() => { setMenuError(null); sessionRef.current?.focus(); }}><CloseIcon size={12} /></button></div>}
    {(ended || recovering) && visible && onRestart && <div className="session-ended" role="status" data-theme={launch.kind === "ssh" ? launch.theme : "graphite"} data-state={status}>
      <span className="led" data-state={status} aria-hidden="true" />
      {production && <span className="prod-chip">{labels.production}</span>}
      <span className="session-ended-text">{recovering ? labels.reconnecting : status === "failed" ? labels.sessionFailed : labels.sessionEnded}</span>
      {recovering ? <button className="ghost-button" type="button" onClick={() => { void controllerRef.current?.cancelRecovery().catch(() => {
        sessionRef.current?.writeLocal(new TextEncoder().encode(`\r\n${labels.operationFailed}\r\n`));
      }); }}>{labels.cancelReconnect}</button>
        : <button ref={restartRef} className="primary-action" type="button" onClick={restart}><RetryIcon size={14} />{launch.kind === "ssh" ? labels.reconnect : labels.restartShell}</button>}
      {!recovering && <span className="session-ended-hint">{production ? labels.pressEnterProduction : labels.pressEnter}</span>}
      {onClose && <button className="ghost-button" type="button" onClick={onClose}><CloseIcon size={14} />{labels.closeTerminal}</button>}
    </div>}
  </div>;
}
