import { useCallback, useEffect, useRef, useState, type CSSProperties, type DragEvent, type FormEvent, type KeyboardEvent, type ReactElement } from "react";
import { copy } from "./i18n";
import { snippetMutationError } from "./snippets/errors";
import { paneStatus, tabStatus, useWorkspace, type TabStatus } from "./workspace";
import { PaneTools, PaneWorkspace } from "./panes/PaneWorkspace";
import { paneLeaves } from "./panes/tree";
import { LocalShellLauncher } from "./terminal/LocalShellLauncher";
import { Titlebar } from "./window/Titlebar";
import { invokeTauri, isTauriAvailable, listenTauriEvent } from "./terminal/bridge";
import type { AgentIdentityView, AgentProvider, BackupImportMode, BackupPreview, CredentialId, CredentialView, GroupId, GroupView, HostId, HostView, LaunchRequestView, LocalShellProfile, TabId, TerminalSessionId, TrustedHostKeyView, TrustChallenge, VaultPreferences, VaultRecoveryCandidate, VaultStatus } from "./terminal/contracts";
import { matchesHostSearch } from "./hosts/search";
import { SftpPanel } from "./sftp/SftpPanel";
import { TransferQueue } from "./transfers/TransferQueue";
import { RemoteEditors } from "./editors/RemoteEditors";
import { useStore } from "zustand";
import { panelPreferences } from "./sftp/panelPreferences";
import type { SftpSessionId } from "./sftp/contracts";
import { AlertIcon, CheckIcon, ChevronRightIcon, CloseIcon, CommandIcon, CopyIcon, DownloadIcon, FolderIcon, KeyIcon, LockIcon, PencilIcon, PlusIcon, SearchIcon, ServerIcon, SettingsIcon, ShieldIcon, SidebarIcon, SnippetIcon, StarIcon, TerminalIcon, TunnelIcon, UnlockIcon, UploadIcon } from "./ui/icons";
import { useConfirm } from "./ui/ConfirmDialog";
import { useMediaQuery } from "./ui/useMediaQuery";
import { hostAddressLabel, hostMonogram, isProductionHost, parseQuickHost, productionSource } from "./hosts/identity";
import { fingerprintChunks, hostKeyFile } from "./hosts/fingerprint";
import { effectiveLayout, resolveLayoutChange } from "./sftp/compactLayout";
import { SnippetManager, type SnippetRunRequest } from "./snippets/SnippetManager";
import { TunnelManager } from "./forwarding/TunnelManager";
import type { Snippet, SnippetGroup } from "./snippets/contracts";
import { decodeSnippet, decodeSnippetGroup } from "./snippets/contracts";
import { CommandPalette } from "./commands/CommandPalette";
import type { CommandAction, WorkspaceCommand } from "./commands/contracts";
import type { PaletteAction } from "./commands/registry";
import { paneInputs } from "./panes/input";
import { routeInput } from "./panes/contracts";
import type { ExactU64 } from "./sftp/contracts";
import { ServerStatusBar } from "./status/ServerStatusBar";
import { useServerStatus } from "./status/useServerStatus";
import { terminalThemeId } from "./themes/contracts";
import { useDesktopPreferences, useTerminalThemes } from "./themes/store";
import { ThemeSettings } from "./themes/ThemeSettings";
import { UpdateCenter, UpdateSettings } from "./update/UpdateSettings";
import { LaunchRequests } from "./launch/LaunchRequests";
import { useWindowVisibility } from "./window/useWindowVisibility";
import { HostChoiceDialog } from "./launch/HostChoiceDialog";
import { sshAuthority } from "./launch/authority";
import type { HelloStatus } from "./terminal/contracts";

type Locale = keyof typeof copy;
const initialVault: VaultStatus = { exists: false, unlocked: false, revision: null, epoch: 0 };
const initialPreferences: VaultPreferences = { auto_lock_minutes: 30, disconnect_on_lock: false };
const emptyHostForm = () => ({
  name: "", address: "", port: "22", username: "", credentialId: "" as CredentialId | "",
  tags: "", favorite: false, groupId: "" as GroupId | "", workingDirectory: "", startupCommand: "",
  encoding: "utf-8", theme: "graphite", agentForwarding: false, production: false,
});
type SftpSessionBinding = { terminal: TerminalSessionId; hostId: HostId; session: SftpSessionId };
type Sheet = "host" | "settings" | "snippets" | "tunnels" | null;
const settingsTabs = ["security", "keys", "credentials", "groups", "desktop", "shortcuts"] as const;
type SettingsTab = typeof settingsTabs[number];
type HostField = "name" | "address" | "port" | "username" | "credentialId";
// Below this workspace width the remote-files panel starts folded and stacks under the terminal.
const COMPACT_WORKSPACE = 760;
const SIDEBAR_HIDDEN_KEY = "capstan.sidebar-hidden";
function readSidebarHidden(): boolean {
  try { return localStorage.getItem(SIDEBAR_HIDDEN_KEY) === "1"; } catch { return false; }
}
function errorCode(error: unknown): string | undefined {
  if (typeof error === "object" && error !== null && "code" in error && typeof error.code === "string") return error.code;
  return undefined;
}

export function App(): ReactElement {
  const locale: Locale = navigator.language.startsWith("zh") ? "zh-CN" : "en";
  const labels = copy[locale];
  const themes = useTerminalThemes((state) => state.themes);
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let active = true;
    void invokeTauri("list_terminal_themes").then((themes) => {
      if (active && Array.isArray(themes)) useTerminalThemes.setState({ themes, loadError: false });
    }).catch(() => { if (active) useTerminalThemes.setState({ loadError: true }); });
    void invokeTauri("load_desktop_preferences", { locale }).then((preferences) => {
      if (active && typeof preferences?.copyOnSelect === "boolean" && typeof preferences.systemNotifications === "boolean"
        && typeof preferences.jumpListRecentHosts === "boolean" && typeof preferences.quakeEnabled === "boolean"
        && (preferences.quakeHotkey === null || typeof preferences.quakeHotkey === "string")
        && Number.isInteger(preferences.quakeHeightPercent)) {
        useDesktopPreferences.setState({ preferences, ready: true, loadError: false });
      }
    }).catch(() => { if (active) useDesktopPreferences.setState({ ready: false, loadError: true }); });
    return () => { active = false; };
  }, [locale]);
  const tabs = useWorkspace((state) => state.tabs);
  const activeTab = useWorkspace((state) => state.activeTab);
  const activeEntry = tabs.find((tab) => tab.id === activeTab);
  const activeLaunch = activeEntry?.launch;
  const statusTerminal = activeLaunch?.kind === "ssh" && activeEntry ? activeEntry.paneStates[activeEntry.focusedPaneId]?.session?.id : undefined;
  const serverStatus = useServerStatus(statusTerminal);
  const addTerminal = useWorkspace((state) => state.addTerminal);
  const addSshTerminal = useWorkspace((state) => state.addSshTerminal);
  const openSshHost = useWorkspace((state) => state.openSshHost);
  const closeTerminal = useWorkspace((state) => state.closeTerminal);
  const moveTab = useWorkspace((state) => state.moveTab);
  const setActiveTab = useWorkspace((state) => state.selectTab);
  const narrow = useMediaQuery("(max-width: 720px)");
  const compactWindow = useMediaQuery("(max-width: 900px)");
  const shellRef = useRef<HTMLElement>(null);
  const [shellWidth, setShellWidth] = useState(0);
  useEffect(() => {
    const element = shellRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => setShellWidth(Math.round(entry.contentRect.width)));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  // Measure the workspace, not the window: a Snap half with the host list open is narrow too.
  const compact = shellWidth > 0 ? shellWidth < COMPACT_WORKSPACE : compactWindow;
  const [confirm, confirmElement] = useConfirm();
  const [focusRequest, setFocusRequest] = useState(0);
  const draggedTabRef = useRef<TabId | null>(null);
  const [dropTab, setDropTab] = useState<TabId | null>(null);
  const panelState = useStore(panelPreferences.store);
  useEffect(() => { if (isTauriAvailable()) void panelPreferences.initialize(); }, []);
  useEffect(() => panelPreferences.prune(tabs.map((tab) => tab.id)), [tabs]);
  const [vault, setVault] = useState<VaultStatus>(initialVault);
  const [hello, setHello] = useState<HelloStatus | null>(null);
  const [vaultBusy, setVaultBusy] = useState(false);
  const masterUnlockRef = useRef<HTMLButtonElement>(null);
  const [preferences, setPreferences] = useState<VaultPreferences>(initialPreferences);
  const [hosts, setHosts] = useState<HostView[]>([]);
  const [credentials, setCredentials] = useState<CredentialView[]>([]);
  const [groups, setGroups] = useState<GroupView[]>([]);
  const [trustedHostKeys, setTrustedHostKeys] = useState<TrustedHostKeyView[]>([]);
  const [challenge, setChallenge] = useState<TrustChallenge | null>(null);
  const [backupPreview, setBackupPreview] = useState<{ token: string; preview: BackupPreview } | null>(null);
  const [recoveryCandidates, setRecoveryCandidates] = useState<VaultRecoveryCandidate[]>([]);
  const [restoringRecovery, setRestoringRecovery] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // Confirmations are receipts, not tasks: they clear themselves after a short read.
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(null), 6000);
    return () => window.clearTimeout(timer);
  }, [notice]);
  const [search, setSearch] = useState("");
  const [credentialName, setCredentialName] = useState("");
  const [agentProvider, setAgentProvider] = useState<AgentProvider>("open_ssh");
  const [agentIdentities, setAgentIdentities] = useState<AgentIdentityView[]>([]);
  const [agentFingerprint, setAgentFingerprint] = useState("");
  const [renamingCredentialId, setRenamingCredentialId] = useState<CredentialId | null>(null);
  const [credentialRename, setCredentialRename] = useState("");
  const [editingHost, setEditingHost] = useState<HostView | null>(null);
  const [editingGroup, setEditingGroup] = useState<GroupView | null>(null);
  const [groupName, setGroupName] = useState("");
  const [groupParentId, setGroupParentId] = useState<GroupId | "">("");
  const [groupProduction, setGroupProduction] = useState(false);
  // Narrow windows show the sidebar as an overlay; wider ones can hide it.
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [sidebarHidden, setSidebarHidden] = useState(readSidebarHidden);
  const [rovingHost, setRovingHost] = useState<HostId | null>(null);
  const [commandCopied, setCommandCopied] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const [compactExpanded, setCompactExpanded] = useState<ReadonlySet<string>>(() => new Set());
  const [fieldErrors, setFieldErrors] = useState<ReadonlySet<HostField>>(() => new Set());
  const [sheet, setSheet] = useState<Sheet>(null);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("security");
  const [dropGroup, setDropGroup] = useState<GroupId | null>(null);
  const sheetRef = useRef<HTMLElement>(null);
  const sheetReturnRef = useRef<HTMLElement | null>(null);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const backupDialogRef = useRef<HTMLDialogElement>(null);
  const lastActivityReport = useRef(0);
  const [form, setForm] = useState(emptyHostForm);
  const [pendingSshSaveRequestId, setPendingSshSaveRequestId] = useState<string | null>(null);
  const [pendingHostChoice, setPendingHostChoice] = useState<{
    requestId: string; display: string; hosts: HostView[];
  } | null>(null);
  const handledLaunchRequests = useRef(new Set<string>());
  const launchContinuations = useRef(new Map<string, () => void>());
  const focusAfterShow = useCallback(() => setFocusRequest((value) => value + 1), []);
  const showCwdFallback = useCallback(() => setNotice(labels.launchCwdFallback), [labels.launchCwdFallback]);
  const windowVisible = useWindowVisibility(focusAfterShow, showCwdFallback);
  useEffect(() => {
    if (vault.unlocked) return;
    const ids = [pendingSshSaveRequestId, pendingHostChoice?.requestId].filter((id): id is string => id !== null && id !== undefined);
    for (const requestId of ids) {
      void invokeTauri("dismiss_launch_request", { requestId }).catch(() => undefined);
      launchContinuations.current.get(requestId)?.();
      launchContinuations.current.delete(requestId);
    }
    setPendingSshSaveRequestId(null);
    setPendingHostChoice(null);
  }, [vault.unlocked, pendingSshSaveRequestId, pendingHostChoice]);
  const [sftpSessions, setSftpSessions] = useState<Record<string, SftpSessionBinding>>({});
  const [snippetGroups, setSnippetGroups] = useState<SnippetGroup[]>([]);
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  const [snippetSearch, setSnippetSearch] = useState("");
  const [snippetRequest, setSnippetRequest] = useState<SnippetRunRequest | undefined>();
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [shellProfiles, setShellProfiles] = useState<LocalShellProfile[]>([]);
  // The palette offers one "New terminal" entry per detected shell; discovery
  // runs when it opens so a newly installed WSL distribution shows up.
  useEffect(() => {
    if (!commandPaletteOpen || !isTauriAvailable()) return;
    let disposed = false;
    void invokeTauri("list_local_shells").then((found) => { if (!disposed && Array.isArray(found)) setShellProfiles(found); }).catch(() => undefined);
    return () => { disposed = true; };
  }, [commandPaletteOpen]);
  const focusTransferHost = useCallback((hostId: string): void => {
    const existing = tabs.find((tab) => tab.launch.kind === "ssh" && tab.launch.host_id === hostId);
    if (existing) {
      setActiveTab(existing.id);
      return;
    }
    const host = hosts.find((candidate) => candidate.id === hostId);
    if (host) addSshTerminal(host.id, host.name, terminalThemeId(host.theme));
  }, [addSshTerminal, hosts, setActiveTab, tabs]);

  const refreshVault = useCallback(async (): Promise<void> => {
    if (!isTauriAvailable()) return;
    try {
      const status = await invokeTauri("vault_status");
      setVault(status);
      const helloStatus = await invokeTauri("vault_hello_status").catch(() => null);
      if (helloStatus && typeof helloStatus.available === "boolean" && typeof helloStatus.enabled === "boolean" && typeof helloStatus.invalidated === "boolean") {
        setHello(helloStatus);
        if (status.unlocked && helloStatus.invalidated) setNotice(labels.hello.invalidated);
      }
      if (status.unlocked) {
        setRecoveryCandidates([]);
        const [nextHosts, nextCredentials, nextGroups, nextTrustedHostKeys, nextPreferences, nextSnippetGroups, nextSnippets] = await Promise.all([
          invokeTauri("list_hosts"), invokeTauri("list_credentials"), invokeTauri("list_groups"),
          invokeTauri("list_trusted_host_keys"), invokeTauri("vault_preferences"),
          invokeTauri("list_snippet_groups"), invokeTauri("list_snippets"),
        ]);
        setHosts(nextHosts);
        setCredentials(nextCredentials);
        setGroups(nextGroups);
        setTrustedHostKeys(nextTrustedHostKeys);
        setPreferences(nextPreferences);
        const decodedSnippetGroups = nextSnippetGroups.map(decodeSnippetGroup);
        const snippetGroupIds = new Set(decodedSnippetGroups.map((group) => group.id));
        setSnippetGroups(decodedSnippetGroups);
        setSnippets(nextSnippets.map((snippet) => decodeSnippet(snippet, snippetGroupIds)));
      } else {
        setBackupPreview(null);
        setRecoveryCandidates([]);
        setHosts([]);
        setCredentials([]);
        setGroups([]);
        setTrustedHostKeys([]);
        setSnippetGroups([]);
        setSnippets([]);
        setSnippetRequest(undefined);
        setCommandPaletteOpen(false);
        setEditingHost(null);
        setEditingGroup(null);
        setForm(emptyHostForm());
        setGroupName("");
        setGroupParentId("");
        setGroupProduction(false);
        setCredentialName("");
        setRenamingCredentialId(null);
        setCredentialRename("");
        setSearch("");
        setSheet(null);
        setPreferences(initialPreferences);
      }
    } catch (cause) {
      const code = errorCode(cause);
      setError(code === "vault_format" || code === "vault_authentication" ? labels.invalidVault : labels.vaultRefreshFailed);
    }
  }, [labels.invalidVault, labels.vaultRefreshFailed, labels.hello.invalidated]);

  useEffect(() => { void refreshVault(); }, [refreshVault]);
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const recordActivity = (): void => {
      const now = Date.now();
      if (now - lastActivityReport.current < 1000) return;
      lastActivityReport.current = now;
      void invokeTauri("record_user_activity").catch(() => undefined);
    };
    const checkAutoLock = (): void => {
      void invokeTauri("check_vault_auto_lock").then((locked) => {
        if (locked) {
          setError(labels.autoLocked);
          void refreshVault();
        }
      }).catch(() => undefined);
    };
    const checkOnResume = (): void => {
      if (document.visibilityState === "visible") checkAutoLock();
    };
    window.addEventListener("pointerdown", recordActivity, true);
    window.addEventListener("keydown", recordActivity, true);
    window.addEventListener("touchstart", recordActivity, true);
    window.addEventListener("focus", checkOnResume);
    document.addEventListener("visibilitychange", checkOnResume);
    const timer = window.setInterval(checkAutoLock, 15_000);
    void listenTauriEvent("vault-auto-locked", () => {
      if (disposed) return;
      setError(labels.autoLocked);
      void refreshVault();
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; }).catch(() => undefined);
    return () => {
      disposed = true;
      window.clearInterval(timer);
      window.removeEventListener("pointerdown", recordActivity, true);
      window.removeEventListener("keydown", recordActivity, true);
      window.removeEventListener("touchstart", recordActivity, true);
      window.removeEventListener("focus", checkOnResume);
      document.removeEventListener("visibilitychange", checkOnResume);
      unlisten?.();
    };
  }, [labels.autoLocked, refreshVault]);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenTauriEvent<TrustChallenge>("ssh-trust-challenge", (request) => setChallenge(request))
      .then((stop) => { if (disposed) stop(); else unlisten = stop; })
      .catch(() => undefined);
    return () => { disposed = true; unlisten?.(); };
  }, []);
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (challenge && !dialog.open) {
      dialog.showModal();
      // Start on the non-accepting action: Reject for a new key, Dismiss for a changed one.
      dialog.querySelector<HTMLButtonElement>(".dialog-actions button")?.focus();
    }
    if (!challenge && dialog.open) dialog.close();
  }, [challenge]);
  useEffect(() => {
    const dialog = backupDialogRef.current;
    if (!dialog) return;
    if (backupPreview && !dialog.open) {
      dialog.showModal();
      dialog.querySelector<HTMLButtonElement>(".dialog-actions button")?.focus();
    }
    if (!backupPreview && dialog.open) dialog.close();
  }, [backupPreview]);

  // The locked vault lives in the sidebar, so it never hides while locked.
  const sidebarCollapsed = sidebarHidden && !narrow && vault.unlocked;
  // Under 720px the sidebar is an overlay that is closed by default.
  const sidebarVisible = narrow ? sidebarOpen : !sidebarCollapsed;
  const showSidebar = (): void => { if (narrow) setSidebarOpen(true); else setSidebarHidden(false); };
  const toggleSidebar = (): void => { if (narrow) setSidebarOpen((open) => !open); else setSidebarHidden((hidden) => !hidden); };
  useEffect(() => {
    try { localStorage.setItem(SIDEBAR_HIDDEN_KEY, sidebarHidden ? "1" : "0"); } catch { /* Preference only. */ }
  }, [sidebarHidden]);
  const focusHostSearch = (): void => {
    showSidebar();
    requestAnimationFrame(() => searchRef.current?.focus());
  };
  const focusFiles = (): boolean => {
    const panel = document.getElementById(`${activeTab}-panel`);
    const target = panel?.querySelector<HTMLElement>(".sftp-table tr.selected .sftp-entry-name, .sftp-entry-name, .sftp-path-field input, .sftp-panel-collapsed");
    target?.focus();
    return !!target;
  };
  // The transfer queue, or the remote-edit queue above it when only that exists.
  const focusQueues = (transfersFirst: boolean): boolean => {
    const target = (transfersFirst ? document.querySelector<HTMLElement>(".transfer-queue:not(.remote-edit-queue) .transfer-queue-heading") : null)
      ?? document.querySelector<HTMLElement>(".transfer-queue .transfer-queue-heading");
    target?.focus();
    return !!target;
  };
  // Panes in F6 order. The terminal keeps F6 for its own programs, so cycling
  // starts from the host list or the files panel; shortcuts reach in from anywhere.
  const cyclePanes = (backward: boolean): void => {
    const panes: Array<{ contains: (element: Element) => boolean; focus: () => boolean }> = [
      { contains: (element) => !!element.closest(".sidebar"), focus: () => {
        if (!sidebarVisible) return false;
        const target = document.querySelector<HTMLElement>(".sidebar .host-open[tabindex='0']") ?? searchRef.current;
        target?.focus();
        return !!target;
      } },
      { contains: (element) => !!element.closest(".terminal-pane-terminal"), focus: () => { setFocusRequest((request) => request + 1); return true; } },
      { contains: (element) => !!element.closest(".sftp-panel, .sftp-panel-collapsed"), focus: focusFiles },
      { contains: (element) => !!element.closest(".transfer-queue"), focus: () => focusQueues(false) },
    ];
    const current = panes.findIndex((pane) => document.activeElement && pane.contains(document.activeElement));
    for (let step = 1; step <= panes.length; step += 1) {
      const next = panes[(current + (backward ? -step : step) + panes.length * 2) % panes.length];
      if (next.focus()) return;
    }
  };
  // Windows Terminal's tab shortcuts plus pane jumps. Captured before xterm
  // sees the keys; plain Ctrl+W/T/K/P stay with the shell (readline uses them).
  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent): void => {
      // Protected confirmation focus also protects against workspace shortcuts.
      const protectedDialog = !!document.querySelector('[role="alertdialog"], [role="dialog"][data-state="open"], dialog[open]');
      // The palette also opens over a sheet, except the host editor, whose
      // unsaved form an action could discard.
      if (sheet && sheet !== "host" && !commandPaletteOpen && !protectedDialog && event.ctrlKey && event.shiftKey && !event.altKey && !event.metaKey
        && event.key.toLocaleLowerCase() === "p") {
        event.preventDefault();
        event.stopPropagation();
        setCommandPaletteOpen(true);
        return;
      }
      if (sheet || commandPaletteOpen || protectedDialog) return;
      if (event.key === "Escape" && sidebarOpen && narrow) { setSidebarOpen(false); return; }
      if (event.key === "F6" && !event.ctrlKey && !event.altKey && !event.metaKey && !event.defaultPrevented
        && !(event.target instanceof Element && event.target.closest(".terminal-pane-terminal"))) {
        event.preventDefault();
        cyclePanes(event.shiftKey);
        return;
      }
      // Windows Terminal's split keys: Alt+Shift+Plus splits right, Alt+Shift+Minus splits down.
      if (event.altKey && event.shiftKey && !event.ctrlKey && !event.metaKey && !event.defaultPrevented && (event.code === "Equal" || event.code === "Minus")) {
        const tab = tabs.find((candidate) => candidate.id === activeTab);
        if (!tab) return;
        event.preventDefault();
        event.stopPropagation();
        useWorkspace.getState().splitPane(tab.id, tab.focusedPaneId, event.code === "Equal" ? "horizontal" : "vertical");
        setFocusRequest((request) => request + 1);
        return;
      }
      if (!event.ctrlKey || event.metaKey || event.defaultPrevented) return;
      const key = event.key.toLocaleLowerCase();
      const index = tabs.findIndex((tab) => tab.id === activeTab);
      let handled = true, focusTerminal = true;
      if (event.shiftKey && !event.altKey && key === "t") addTerminal();
      else if (event.shiftKey && !event.altKey && key === "w") closeTerminal(activeTab);
      else if (event.shiftKey && !event.altKey && key === "k" && vault.unlocked) { focusHostSearch(); focusTerminal = false; }
      else if (event.shiftKey && !event.altKey && key === "p") { setCommandPaletteOpen(true); focusTerminal = false; }
      else if (event.shiftKey && !event.altKey && key === "b") { toggleSidebar(); focusTerminal = false; }
      else if (event.shiftKey && !event.altKey && key === "e") { handled = focusFiles(); focusTerminal = false; }
      else if (event.shiftKey && !event.altKey && key === "j") { handled = focusQueues(true); focusTerminal = false; }
      else if (!event.shiftKey && !event.altKey && key === "," && vault.unlocked) { openSheet("settings"); focusTerminal = false; }
      else if (!event.altKey && key === "tab") setActiveTab(tabs[(index + (event.shiftKey ? -1 : 1) + tabs.length) % tabs.length].id);
      else if (event.altKey && !event.shiftKey && /^[1-9]$/u.test(event.key)) {
        const target = event.key === "9" ? tabs.at(-1) : tabs[Number(event.key) - 1];
        if (target) setActiveTab(target.id); else handled = false;
      } else handled = false;
      if (!handled) return;
      event.preventDefault();
      event.stopPropagation();
      if (focusTerminal) setFocusRequest((request) => request + 1);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  });
  useEffect(() => {
    const element = sheetRef.current;
    if (!element) return;
    const target = sheet === "host" ? element.querySelector<HTMLElement>("input")
      : sheet === "snippets" ? element.querySelector<HTMLElement>('input[type="search"]')
        : sheet === "tunnels" ? element.querySelector<HTMLElement>("button")
        : element.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
    (target ?? element).focus();
  }, [sheet]);
  const openSheet = (kind: Exclude<Sheet, null>): void => {
    if (sheet === "host" && kind !== "host" && pendingSshSaveRequestId) {
      void dismissLaunchRequest(pendingSshSaveRequestId);
      setPendingSshSaveRequestId(null);
    }
    if (!sheet) sheetReturnRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setSheet(kind);
  };
  const closeSheet = (): void => {
    setSheet(null);
    const target = sheetReturnRef.current;
    sheetReturnRef.current = null;
    if (target?.isConnected) requestAnimationFrame(() => target.focus());
  };
  const showSnippetMutationError = async (cause: unknown): Promise<void> => {
    const code = errorCode(cause);
    if (code === "vault_conflict" || code === "not_found" || code === "vault_locked") await refreshVault();
    setNotice(null);
    setError(snippetMutationError(cause, labels));
  };
  const saveSnippetGroup = async (group: SnippetGroup): Promise<void> => {
    try {
      await invokeTauri("save_snippet_group", {
        group,
        expectedRevision: String(vault.revision ?? 0) as ExactU64,
      });
      setNotice(labels.snippetGroupSaved);
      await refreshVault();
    } catch (cause) { await showSnippetMutationError(cause); throw cause; }
  };
  const deleteSnippetGroup = async (id: string): Promise<void> => {
    const name = snippetGroups.find((group) => group.id === id)?.name ?? "";
    if (!await confirm({
      title: labels.confirmDeleteSnippetGroupTitle(name), body: labels.confirmDeleteSnippetGroup,
      confirm: labels.deleteNamed(name), cancel: labels.cancelEdit, tone: "danger",
    })) return;
    try {
      await invokeTauri("delete_snippet_group", {
        id,
        expectedRevision: String(vault.revision ?? 0) as ExactU64,
      });
      setNotice(labels.snippetGroupDeleted);
      await refreshVault();
    } catch (cause) { await showSnippetMutationError(cause); throw cause; }
  };
  const saveSnippet = async (snippet: Snippet): Promise<void> => {
    try {
      await invokeTauri("save_snippet", {
        snippet,
        expectedRevision: String(vault.revision ?? 0) as ExactU64,
      });
      setNotice(labels.snippetSaved);
      await refreshVault();
    } catch (cause) { await showSnippetMutationError(cause); throw cause; }
  };
  const deleteSnippet = async (id: string): Promise<void> => {
    const name = snippets.find((snippet) => snippet.id === id)?.name ?? labels.snippetUntitled;
    if (!await confirm({
      title: labels.confirmDeleteSnippetTitle(name), body: labels.confirmDeleteSnippet,
      confirm: labels.deleteNamed(name), cancel: labels.cancelEdit, tone: "danger",
    })) return;
    try {
      await invokeTauri("delete_snippet", {
        id,
        expectedRevision: String(vault.revision ?? 0) as ExactU64,
      });
      setNotice(labels.snippetDeleted);
      await refreshVault();
    } catch (cause) { await showSnippetMutationError(cause); throw cause; }
  };
  const runSnippetInput = async (input: string, target: "current" | "broadcast"): Promise<void> => {
    setError(null);
    setNotice(null);
    const current = useWorkspace.getState();
    const tab = current.tabs.find((candidate) => candidate.id === current.activeTab);
    if (!tab) return;
    const broadcast = target === "broadcast";
    const route = {
      ...routeInput(tab.focusedPaneId, tab.broadcastPaneIds, broadcast, input),
      // Every snippet broadcast needs consent, even without a trailing Enter.
      requiresConfirmation: broadcast,
    };
    if (route.blockedReason) { setError(labels.snippetNoBroadcastTargets); return; }
    const targetNames = route.targetPaneIds.map((id) => {
      const index = paneLeaves(tab.root).findIndex((pane) => pane.paneId === id) + 1;
      return labels.paneName(tabNames.get(tab.id) ?? labels.localShell, index || 0);
    });
    // Capture destination session owners before the confirmation yields.
    // Auto-reconnect can replace an endpoint without changing PaneId or launch.
    const send = paneInputs.prepare(route);
    const targetLaunches = new Map(route.targetPaneIds.map((id) => [id, paneLeaves(tab.root).find((pane) => pane.paneId === id)?.launch]));
    const result = await send(
      new TextEncoder().encode(input),
      // Snippets are confirmed on every broadcast send, armed or not.
      async () => confirm({
        title: labels.confirmSnippetBroadcastTitle(route.targetPaneIds.length),
        body: `${labels.confirmBroadcastMessage(route.targetPaneIds.length)}\n${targetNames.join(labels.paneListSeparator)}`,
        confirm: labels.confirmBroadcastSend(route.targetPaneIds.length),
        cancel: labels.cancelEdit,
        tone: "danger",
      }),
      () => {
        const latest = useWorkspace.getState();
        const latestTab = latest.tabs.find((candidate) => candidate.id === tab.id);
        return !!latestTab && latest.activeTab === tab.id
          && (broadcast || latestTab.focusedPaneId === tab.focusedPaneId)
          && route.targetPaneIds.every((id) => paneLeaves(latestTab.root).some((pane) => pane.paneId === id && pane.launch === targetLaunches.get(id)));
      },
    );
    if (result.failedTargets.length > 0) {
      setNotice(null);
      const failed = result.failedTargets.map(({ paneId, errorCode }) => {
        const index = paneLeaves(tab.root).findIndex((pane) => pane.paneId === paneId) + 1;
        const pane = labels.paneName(tabNames.get(tab.id) ?? labels.localShell, index || 0);
        const reason = errorCode === "input_queue_full" || errorCode === "queue_full" ? labels.paneInputQueueFull
          : errorCode === "session_closed" || errorCode === "not_found" ? labels.paneInputClosed : labels.paneInputUnknown;
        return `${pane} (${reason})`;
      });
      setError(`${labels.snippetSendFailed} ${failed.join(labels.paneListSeparator)}`);
    } else if (result.deliveredPaneIds.length > 0) {
      setError(null);
      setNotice(labels.snippetSent);
    }
  };
  const addHost = (prefill?: { name: string; address: string; port: string; username: string }): void => {
    if (!prefill && pendingSshSaveRequestId) {
      void dismissLaunchRequest(pendingSshSaveRequestId);
      setPendingSshSaveRequestId(null);
    }
    setEditingHost(null);
    setFieldErrors(new Set());
    setForm({ ...emptyHostForm(), ...prefill });
    openSheet("host");
  };
  const cancelHostEdit = (): void => {
    const launchRequestId = pendingSshSaveRequestId;
    setPendingSshSaveRequestId(null);
    if (launchRequestId) void dismissLaunchRequest(launchRequestId);
    setEditingHost(null);
    setFieldErrors(new Set());
    setForm(emptyHostForm());
    closeSheet();
  };
  const openHost = (host: HostView, newTab = false): void => {
    openSshHost(host.id, host.name, terminalThemeId(host.theme), newTab);
    setSidebarOpen(false);
    setFocusRequest((request) => request + 1);
  };
  const dismissLaunchRequest = async (requestId: string): Promise<void> => {
    try { await invokeTauri("dismiss_launch_request", { requestId }); } catch { /* Expired requests are already gone. */ }
    launchContinuations.current.get(requestId)?.();
    launchContinuations.current.delete(requestId);
  };
  const unlockHostsForLaunch = async (): Promise<HostView[]> => {
    const status = await invokeTauri("vault_status");
    if (!status.unlocked) {
      if (status.exists) await invokeTauri("unlock_vault", { locale });
      else await invokeTauri("initialize_vault", { locale });
    }
    await refreshVault();
    const currentHosts = await invokeTauri("list_hosts");
    setHosts(currentHosts);
    return currentHosts;
  };
  const processLaunchRequest = async (request: LaunchRequestView): Promise<void> => {
    if (handledLaunchRequests.current.has(request.requestId)) return;
    handledLaunchRequests.current.add(request.requestId);
    try {
      const details = await invokeTauri("inspect_launch_request", { requestId: request.requestId });
      if (request.kind === "focus" || details.kind === "focus") {
        await dismissLaunchRequest(request.requestId);
        return;
      }
      if (request.kind === "local" && details.kind === "local") {
        addTerminal({ launch_request_id: request.requestId }, request.display || labels.localShell);
        return;
      }
      if (request.kind === "ssh" && details.kind === "ssh") {
        const target = details.target;
        const authority = sshAuthority(target.username, target.host, target.port);
        if (!await confirm({
          title: labels.sshLinkTitle,
          body: labels.sshLinkConfirm(target.username ?? labels.sshUserUnspecified, target.host, target.port),
          confirm: labels.sshLinkContinue, cancel: labels.cancelEdit, tone: "primary",
        })) {
          await dismissLaunchRequest(request.requestId);
          return;
        }
        const currentHosts = await unlockHostsForLaunch();
        const matchingIds = new Set(await invokeTauri("match_launch_hosts", { requestId: request.requestId }));
        const matches = currentHosts.filter((host) => matchingIds.has(host.id));
        if (matches.length === 1) {
          openHost(matches[0]);
          await dismissLaunchRequest(request.requestId);
        } else if (matches.length > 1) {
          await new Promise<void>((resolve) => {
            launchContinuations.current.set(request.requestId, resolve);
            setPendingHostChoice({ requestId: request.requestId, display: authority, hosts: matches });
          });
        } else {
          await new Promise<void>((resolve) => {
            launchContinuations.current.set(request.requestId, resolve);
            setPendingSshSaveRequestId(request.requestId);
            addHost({
              name: target.host, address: target.host, port: String(target.port), username: target.username ?? "",
            });
          });
        }
        return;
      }
      if (request.kind === "host" && details.kind === "host") {
        const currentHosts = await unlockHostsForLaunch();
        const host = currentHosts.find((candidate) => candidate.id === details.hostId);
        if (host) {
          if (await confirm({ title: labels.sshLinkTitle,
            body: labels.sshLinkConfirm(host.username, host.address, host.port),
            confirm: labels.sshLinkContinue, cancel: labels.cancelEdit, tone: "primary" })) openHost(host);
        } else setNotice(labels.launchHostMissing);
        await dismissLaunchRequest(request.requestId);
        return;
      }
      await dismissLaunchRequest(request.requestId);
    } catch (cause) {
      await dismissLaunchRequest(request.requestId);
      const code = errorCode(cause);
      if (code !== "canceled") setError(labels.launchRequestFailed);
      else setNotice(labels.launchUnlockCanceled);
    } finally {
      handledLaunchRequests.current.delete(request.requestId);
    }
  };
  const cancelLaunchHostChoice = (): void => {
    const requestId = pendingHostChoice?.requestId;
    setPendingHostChoice(null);
    if (requestId) void dismissLaunchRequest(requestId);
  };
  const chooseLaunchHost = (hostId: HostId): void => {
    const pending = pendingHostChoice;
    if (!pending) return;
    const host = pending.hosts.find((candidate) => candidate.id === hostId);
    setPendingHostChoice(null);
    if (host) openHost(host);
    void dismissLaunchRequest(pending.requestId);
  };
  const withFocus = (action: () => void) => (): void => {
    action();
    setFocusRequest((request) => request + 1);
  };
  const groupNames = new Map<string, string>(groups.map((group) => [group.id, group.name]));
  const groupsById = new Map<string, GroupView>(groups.map((group) => [group.id, group]));
  const groupSortKey = (host: HostView): string | null => host.group_id ? groupNames.get(host.group_id) ?? null : null;
  const visibleHosts = hosts
    .filter((host) => matchesHostSearch(host, groupNames, search))
    .sort((left, right) => {
      if (left.favorite !== right.favorite) return Number(right.favorite) - Number(left.favorite);
      // Ungrouped hosts follow the named groups.
      const leftGroup = left.favorite ? "" : groupSortKey(left), rightGroup = right.favorite ? "" : groupSortKey(right);
      if (leftGroup === null || rightGroup === null) { if (leftGroup !== rightGroup) return leftGroup === null ? 1 : -1; }
      else if (leftGroup !== rightGroup) return leftGroup.localeCompare(rightGroup);
      return left.name.localeCompare(right.name);
    });
  const quickHost = visibleHosts.length === 0 ? parseQuickHost(search) : null;
  const credentialKindLabel = (kind: string): string => ({
    password: labels.credentialKindPassword,
    imported_key: labels.credentialKindImportedKey,
    key_file: labels.credentialKindKeyFile,
    agent: labels.credentialKindAgent,
  }[kind] ?? kind);
  const showError = (cause: unknown): void => {
    const code = errorCode(cause);
    if (code === "host_key_changed") setError(labels.hostKeyChanged);
    else if (code === "hello_unavailable") setError(labels.hello.unavailable);
    else if (code === "hello_canceled") setError(labels.hello.canceled);
    else if (code === "hello_key_missing") setError(labels.hello.keyMissing);
    else if (code === "hello_mismatch") setError(labels.hello.mismatch);
    else if (code === "hello_data") setError(labels.hello.data);
    else if (code === "ssh_authentication") setError(labels.authenticationFailed);
    else if (code === "canceled") setError(labels.operationCanceled);
    else if (code === "invalid_input") setError(labels.invalidInput);
    else if (code === "group_in_use") setError(labels.groupInUse);
    else if (code === "credential_in_use") setError(labels.credentialInUse);
    else if (code === "ssh_private_key_invalid") setError(labels.privateKeyInvalid);
    else if (code === "ssh_agent_unavailable") setError(labels.agentUnavailable);
    else if (code === "ssh_agent_identity_unavailable") setError(labels.agentIdentityUnavailable);
    else if (code === "trust_conflict") setError(labels.backupTrustConflict);
    else if (code === "backup_import_expired") setError(labels.backupPreviewExpired);
    else if (code === "credential_file") setError(labels.credentialFileInvalid);
    else if (code === "vault_kdf_policy") setError(labels.vaultPasswordPolicy);
    else if (code === "vault_recovery_expired") setError(labels.recoverySelectionExpired);
    else if (code === "vault_recovery_limit") setError(labels.recoveryCandidateLimit);
    else if (code === "vault_data") setError(labels.invalidVaultData);
    else setError(labels.connectionFailed);
  };

  const runVaultAction = async (action: "initialize_vault" | "unlock_vault"): Promise<void> => {
    setError(null);
    setVaultBusy(true);
    try {
      await invokeTauri(action, { locale });
      setRecoveryCandidates([]);
      await refreshVault();
    } catch (cause) {
      const code = errorCode(cause);
      if (code === "vault_authentication" || code === "vault_format" || code === "vault_kdf_policy") setError(labels.invalidVault);
      else if (code === "canceled") setError(labels.operationCanceled);
      else showError(cause);
    } finally { setVaultBusy(false); }
  };
  const runHelloAction = async (action: "enable_vault_hello" | "disable_vault_hello" | "unlock_vault_hello"): Promise<void> => {
    if (vaultBusy) return;
    setError(null);
    setNotice(null);
    setVaultBusy(true);
    try {
      if (action === "enable_vault_hello") await invokeTauri(action, { locale });
      else await invokeTauri(action);
      await refreshVault();
      if (action === "enable_vault_hello") setNotice(labels.hello.saved);
      else if (action === "disable_vault_hello") setNotice(labels.hello.removed);
    } catch (cause) {
      if (errorCode(cause) === "vault_authentication") setError(labels.invalidVault);
      else if (errorCode(cause) === "vault_io") setError(labels.hello.storage);
      else showError(cause);
      if (action === "unlock_vault_hello") requestAnimationFrame(() => masterUnlockRef.current?.focus());
    } finally { setVaultBusy(false); }
  };
  const findRecoveryCandidates = async (): Promise<void> => {
    setError(null);
    setNotice(null);
    try {
      const candidates = await invokeTauri("list_vault_recovery_candidates", { locale });
      setRecoveryCandidates(candidates);
      if (candidates.length === 0) setError(labels.noRecoveryCandidates);
      else setNotice(labels.recoveryCandidatesFound(candidates.length));
    } catch (cause) {
      if (errorCode(cause) === "canceled") setError(labels.operationCanceled);
      else if (errorCode(cause) === "vault_authentication" || errorCode(cause) === "vault_format") setError(labels.recoveryPasswordInvalid);
      else showError(cause);
    }
  };
  const restoreRecoveryCandidate = async (candidate: VaultRecoveryCandidate): Promise<void> => {
    if (!await confirm({
      title: labels.confirmRecoveryTitle, body: labels.confirmRecovery(candidate.revision),
      confirm: labels.restoreRevision(candidate.revision), cancel: labels.cancelEdit,
    })) return;
    setError(null);
    setNotice(null);
    setRestoringRecovery(candidate.token);
    try {
      await invokeTauri("restore_vault_recovery_candidate", { token: candidate.token, locale });
      setRecoveryCandidates([]);
      setNotice(labels.vaultRecovered);
      await refreshVault();
    } catch (cause) {
      if (errorCode(cause) === "canceled") setError(labels.operationCanceled);
      else if (errorCode(cause) === "vault_authentication" || errorCode(cause) === "vault_format") setError(labels.recoveryPasswordInvalid);
      else showError(cause);
    } finally {
      setRestoringRecovery(null);
    }
  };
  const createCredential = async (): Promise<void> => {
    const name = credentialName.trim();
    if (!name) return;
    setError(null);
    try {
      const id = await invokeTauri("create_password_credential", { name, locale });
      setCredentialName("");
      await refreshVault();
      setForm((current) => ({ ...current, credentialId: id }));
    } catch (cause) { showError(cause); }
  };
  const importPrivateKey = async (): Promise<void> => {
    const name = credentialName.trim();
    if (!name) return;
    setError(null);
    try {
      const id = await invokeTauri("import_private_key_credential", { name, locale });
      setCredentialName("");
      await refreshVault();
      setForm((current) => ({ ...current, credentialId: id }));
    } catch (cause) { showError(cause); }
  };
  const linkPrivateKeyFile = async (): Promise<void> => {
    const name = credentialName.trim();
    if (!name) return;
    setError(null);
    try {
      const id = await invokeTauri("add_private_key_file_credential", { name, locale });
      setCredentialName("");
      await refreshVault();
      setForm((current) => ({ ...current, credentialId: id }));
    } catch (cause) { showError(cause); }
  };
  const loadAgentIdentities = async (): Promise<void> => {
    setError(null);
    try {
      const identities = await invokeTauri("list_agent_identities", { provider: agentProvider });
      setAgentIdentities(identities);
      setAgentFingerprint(identities[0]?.fingerprint ?? "");
      if (identities.length === 0) setError(labels.agentNoIdentities);
    } catch (cause) { showError(cause); }
  };
  const addAgentCredential = async (): Promise<void> => {
    const name = credentialName.trim();
    if (!name || !agentFingerprint) return;
    setError(null);
    try {
      const id = await invokeTauri("create_agent_credential", {
        name, agent: agentProvider, fingerprint: agentFingerprint,
      });
      setCredentialName("");
      await refreshVault();
      setForm((current) => ({ ...current, credentialId: id }));
    } catch (cause) { showError(cause); }
  };
  const beginRenameCredential = (credential: CredentialView): void => {
    setRenamingCredentialId(credential.id);
    setCredentialRename(credential.name);
    setError(null);
  };
  const renameCredential = async (credential: CredentialView): Promise<void> => {
    const name = credentialRename.trim();
    if (!name) { setError(labels.invalidInput); return; }
    setError(null);
    try {
      await invokeTauri("rename_credential", {
        id: credential.id, name, expectedRevision: vault.revision ?? 0,
      });
      setRenamingCredentialId(null);
      setCredentialRename("");
      setNotice(labels.credentialRenamed);
      await refreshVault();
    } catch (cause) { showError(cause); }
  };
  const updatePasswordCredential = async (credential: CredentialView): Promise<void> => {
    if (credential.kind !== "password") return;
    setError(null);
    try {
      await invokeTauri("update_password_credential", {
        id: credential.id, expectedRevision: vault.revision ?? 0, locale,
      });
      setNotice(labels.credentialUpdated);
      await refreshVault();
    } catch (cause) { showError(cause); }
  };
  const deleteCredential = async (credential: CredentialView): Promise<void> => {
    if (!await confirm({
      title: labels.confirmDeleteCredentialTitle, body: labels.confirmDeleteCredential(credential.name),
      confirm: labels.deleteNamed(credential.name), cancel: labels.cancelEdit,
    })) return;
    setError(null);
    try {
      await invokeTauri("delete_credential", {
        id: credential.id, expectedRevision: vault.revision ?? 0,
      });
      if (form.credentialId === credential.id) setForm((current) => ({ ...current, credentialId: "" }));
      if (renamingCredentialId === credential.id) {
        setRenamingCredentialId(null);
        setCredentialRename("");
      }
      setNotice(labels.credentialDeleted);
      await refreshVault();
    } catch (cause) { showError(cause); }
  };
  const saveHost = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    const port = Number(form.port);
    const missing = new Set<HostField>();
    if (!form.name.trim()) missing.add("name");
    if (!form.address.trim()) missing.add("address");
    if (!form.username.trim()) missing.add("username");
    if (!form.credentialId) missing.add("credentialId");
    if (!Number.isInteger(port) || port < 1 || port > 65535) missing.add("port");
    setFieldErrors(missing);
    const credentialId = form.credentialId;
    if (missing.size || !credentialId) {
      requestAnimationFrame(() => document.querySelector<HTMLElement>(".host-editor [aria-invalid='true']")?.focus());
      return;
    }
    const host: HostView = {
      id: editingHost?.id ?? (globalThis.crypto.randomUUID() as HostId),
      name: form.name.trim(), address: form.address.trim(), port, username: form.username.trim(),
      credential_id: credentialId, group_id: form.groupId || null,
      tags: form.tags.split(",").map((tag) => tag.trim()).filter(Boolean), favorite: form.favorite,
      revision: editingHost?.revision ?? 0,
      working_directory: form.workingDirectory.trim() || null,
      startup_command: form.startupCommand.trim() || null,
      encoding: form.encoding, theme: form.theme, agent_forwarding: form.agentForwarding, production: form.production,
    };
    setError(null);
    try {
      await invokeTauri("save_host", { host, expectedRevision: vault.revision ?? 0 });
      const launchRequestId = pendingSshSaveRequestId;
      setPendingSshSaveRequestId(null);
      setEditingHost(null);
      setForm(emptyHostForm());
      closeSheet();
      await refreshVault();
      if (launchRequestId) {
        openHost(host);
        await dismissLaunchRequest(launchRequestId);
        setNotice(labels.sshLinkHostSaved);
      }
    } catch (cause) { showError(cause); }
  };
  const editHost = (host: HostView): void => {
    if (pendingSshSaveRequestId) {
      void dismissLaunchRequest(pendingSshSaveRequestId);
      setPendingSshSaveRequestId(null);
    }
    setEditingHost(host);
    setFieldErrors(new Set());
    setForm({
      name: host.name, address: host.address, port: String(host.port), username: host.username,
      credentialId: host.credential_id, tags: host.tags.join(", "), favorite: host.favorite,
      groupId: host.group_id ?? "",
      workingDirectory: host.working_directory ?? "", startupCommand: host.startup_command ?? "",
      encoding: host.encoding, theme: host.theme, agentForwarding: host.agent_forwarding, production: host.production,
    });
    openSheet("host");
  };
  const deleteHost = async (host: HostView): Promise<void> => {
    if (!await confirm({
      title: labels.confirmDeleteHostTitle, body: labels.confirmDeleteHost(host.name),
      confirm: labels.deleteNamed(host.name), cancel: labels.cancelEdit,
    })) return;
    try {
      await invokeTauri("delete_host", { id: host.id, expectedRevision: vault.revision ?? 0 });
      if (editingHost?.id === host.id) { setEditingHost(null); setForm((current) => ({ ...current, name: "", address: "", username: "" })); closeSheet(); }
      await refreshVault();
    } catch (cause) { showError(cause); }
  };
  const moveHostToGroup = async (hostId: HostId, groupId: GroupId | null): Promise<void> => {
    const host = hosts.find((candidate) => candidate.id === hostId);
    if (!host || host.group_id === groupId) return;
    try {
      await invokeTauri("save_host", {
        host: { ...host, group_id: groupId },
        expectedRevision: vault.revision ?? 0,
      });
      await refreshVault();
    } catch (cause) { showError(cause); }
  };
  const moveGroupToParent = async (group: GroupView, parent: GroupId): Promise<void> => {
    if (group.id === parent || group.parent_id === parent) return;
    try {
      await invokeTauri("save_group", {
        group: { ...group, parent_id: parent },
        expectedRevision: vault.revision ?? 0,
      });
      await refreshVault();
    } catch (cause) { showError(cause); }
  };
  const forgetTrustedHostKey = async (trusted: TrustedHostKeyView): Promise<void> => {
    if (!await confirm({
      title: labels.confirmForgetKeyTitle, body: labels.confirmForgetTrustedKey(trusted.target, trusted.port, trusted.fingerprint),
      confirm: labels.forgetNamed(`${trusted.target}:${trusted.port}`), cancel: labels.cancelEdit,
    })) return;
    setError(null);
    setNotice(null);
    try {
      await invokeTauri("forget_trusted_host_key", {
        target: trusted.target, port: trusted.port, fingerprint: trusted.fingerprint,
        expectedRevision: vault.revision ?? 0,
      });
      setNotice(labels.trustedHostKeyForgotten(trusted.target, trusted.port));
      await refreshVault();
    } catch (cause) { showError(cause); }
  };
  const dropOnGroup = (event: DragEvent<HTMLLIElement>, group: GroupView): void => {
    event.preventDefault();
    const hostId = event.dataTransfer.getData("application/x-capstan-host") as HostId;
    if (hostId) {
      void moveHostToGroup(hostId, group.id);
      return;
    }
    const groupId = event.dataTransfer.getData("application/x-capstan-group") as GroupId;
    const dragged = groups.find((candidate) => candidate.id === groupId);
    if (dragged) void moveGroupToParent(dragged, group.id);
  };
  const groupDepth = (group: GroupView): number => {
    let depth = 0;
    let parentId = group.parent_id;
    while (parentId) {
      const parent = groups.find((candidate) => candidate.id === parentId);
      if (!parent) break;
      depth += 1;
      parentId = parent.parent_id;
    }
    return depth;
  };
  const saveGroup = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    const name = groupName.trim();
    if (!name) return;
    const group: GroupView = {
      id: editingGroup?.id ?? (globalThis.crypto.randomUUID() as GroupId),
      name,
      parent_id: groupParentId || null,
      production: groupProduction,
    };
    try {
      await invokeTauri("save_group", { group, expectedRevision: vault.revision ?? 0 });
      setEditingGroup(null);
      setGroupName("");
      setGroupParentId("");
      setGroupProduction(false);
      await refreshVault();
    } catch (cause) { showError(cause); }
  };
  const editGroup = (group: GroupView): void => {
    setEditingGroup(group);
    setGroupName(group.name);
    setGroupParentId(group.parent_id ?? "");
    setGroupProduction(group.production);
  };
  const deleteGroup = async (group: GroupView): Promise<void> => {
    if (!await confirm({
      title: labels.confirmDeleteGroupTitle, body: labels.confirmDeleteGroup(group.name),
      confirm: labels.deleteNamed(group.name), cancel: labels.cancelEdit,
    })) return;
    try {
      await invokeTauri("delete_group", { id: group.id, expectedRevision: vault.revision ?? 0 });
      if (editingGroup?.id === group.id) { setEditingGroup(null); setGroupName(""); setGroupParentId(""); setGroupProduction(false); }
      await refreshVault();
    } catch (cause) { showError(cause); }
  };
  const respondToChallenge = async (accepted: boolean): Promise<void> => {
    if (!challenge) return;
    if (challenge.keyChanged) { setChallenge(null); return; }
    try {
      await invokeTauri("answer_host_trust", { challengeId: challenge.challengeId, accepted });
      setChallenge(null);
    } catch (cause) { setChallenge(null); showError(cause); }
  };
  const lockVault = async (): Promise<void> => {
    try { await invokeTauri("lock_vault"); setChallenge(null); await refreshVault(); }
    catch (cause) { showError(cause); }
  };
  const saveVaultSettings = async (): Promise<void> => {
    if (preferences.auto_lock_minutes !== null
      && (!Number.isInteger(preferences.auto_lock_minutes) || preferences.auto_lock_minutes < 1 || preferences.auto_lock_minutes > 1440)) {
      setError(labels.autoLockRangeError);
      return;
    }
    try {
      const revision = await invokeTauri("update_vault_preferences", {
        autoLockMinutes: preferences.auto_lock_minutes,
        disconnectOnLock: preferences.disconnect_on_lock,
        expectedRevision: vault.revision ?? 0,
      });
      setVault((current) => ({ ...current, revision }));
      setError(null);
      setNotice(labels.vaultSettingsSaved);
    } catch (cause) { setNotice(null); showError(cause); }
  };
  const changeMasterPassword = async (): Promise<void> => {
    try {
      await invokeTauri("change_vault_master_password", { expectedRevision: vault.revision ?? 0, locale });
      await refreshVault();
      setNotice(labels.masterPasswordChanged);
    } catch (cause) { showError(cause); }
  };
  const exportVaultBackup = async (): Promise<void> => {
    try {
      await invokeTauri("export_vault_backup", { locale });
      setNotice(labels.backupExported);
    } catch (cause) { showError(cause); }
  };
  const previewVaultBackup = async (): Promise<void> => {
    try {
      const result = await invokeTauri("preview_vault_backup", {
        expectedRevision: vault.revision ?? 0, expectedEpoch: vault.epoch, locale,
      });
      setBackupPreview(result);
      setError(null);
    } catch (cause) { showError(cause); }
  };
  const cancelBackupPreview = (): void => {
    if (backupPreview) void invokeTauri("cancel_vault_backup_preview", { token: backupPreview.token }).catch(() => undefined);
    setBackupPreview(null);
  };
  const importVaultBackup = async (mode: BackupImportMode): Promise<void> => {
    if (!backupPreview) return;
    try {
      await invokeTauri("import_vault_backup", {
        token: backupPreview.token, mode,
        expectedRevision: backupPreview.preview.baseRevision,
        expectedEpoch: vault.epoch,
      });
      setBackupPreview(null);
      setNotice(labels.backupImported);
      await refreshVault();
    } catch (cause) { setBackupPreview(null); showError(cause); }
  };

  const hostSignals = new Map<string, TabStatus>();
  for (const tab of tabs) {
    for (const pane of paneLeaves(tab.root)) {
      if (pane.launch.kind !== "ssh") continue;
      const status = paneStatus(tab, pane.paneId), previous = hostSignals.get(pane.launch.host_id);
      if (status === "live" || (status === "connecting" && previous !== "live")) hostSignals.set(pane.launch.host_id, status);
    }
  }
  const productionHosts = new Set(hosts.filter((host) => isProductionHost(host, groupsById)).map((host) => host.id));
  const hostSections: { label: string; favorites: boolean; hosts: HostView[] }[] = [];
  for (const host of visibleHosts) {
    const label = host.favorite ? labels.favoriteHosts : groupNames.get(host.group_id ?? "") ?? labels.noGroup;
    const last = hostSections.at(-1);
    if (last && last.label === label && last.favorites === host.favorite) last.hosts.push(host);
    else hostSections.push({ label, favorites: host.favorite, hosts: [host] });
  }
  // Unlabeled local tabs are numbered among themselves, not by tab position.
  const tabNames = new Map<string, string>();
  let localOrdinal = 0;
  for (const tab of tabs) tabNames.set(tab.id, tab.label ?? labels.powerShellTab(++localOrdinal));
  const activeHost = activeLaunch?.kind === "ssh" ? hosts.find((host) => host.id === activeLaunch.host_id) : undefined;
  const activePaneTheme = activeEntry?.paneStates[activeEntry.focusedPaneId]?.themeOverride
    ?? (activeLaunch?.kind === "ssh" ? activeLaunch.theme : "graphite");
  const titleContext = activeHost ? `${activeHost.username}@${activeHost.address}`
    : activeEntry ? tabNames.get(activeEntry.id) ?? labels.localShell : labels.localShell;
  const challengeHost = challenge ? hosts.find((host) => host.id === challenge.hostId) : undefined;
  const challengeProduction = !!challengeHost && productionHosts.has(challengeHost.id);
  const verifyCommand = challenge ? `ssh-keygen -lf ${hostKeyFile(challenge.algorithm)}` : "";
  const copyVerifyCommand = (): void => {
    void navigator.clipboard?.writeText(verifyCommand).then(() => {
      setCommandCopied(true);
      window.setTimeout(() => setCommandCopied(false), 1600);
    }).catch(() => undefined);
  };
  const fingerprint = (value: string, other: string | null): ReactElement => <span className="fingerprint">
    {fingerprintChunks(value, other).map((chunk, index) => <span key={index} className={chunk.changed ? "fp-chunk fp-changed" : "fp-chunk"}>{chunk.text}</span>)}
  </span>;
  const firstHost = visibleHosts[0]?.id ?? null;
  const tabbableHost = rovingHost && visibleHosts.some((host) => host.id === rovingHost) ? rovingHost : firstHost;
  const hostRowKeyDown = (event: KeyboardEvent<HTMLButtonElement>, host: HostView): void => {
    if (event.key === "F2") { event.preventDefault(); editHost(host); return; }
    if (event.key === "Enter" && event.ctrlKey) { event.preventDefault(); openHost(host, true); return; }
    if (event.key === "Escape") { event.preventDefault(); setFocusRequest((request) => request + 1); return; }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const rows = [...document.querySelectorAll<HTMLButtonElement>(".sidebar .host-open")];
    const index = rows.indexOf(event.currentTarget);
    if (event.key === "ArrowUp" && index === 0) { searchRef.current?.focus(); return; }
    const next = event.key === "Home" ? rows[0] : event.key === "End" ? rows.at(-1)
      : rows[Math.min(rows.length - 1, Math.max(0, index + (event.key === "ArrowDown" ? 1 : -1)))];
    next?.focus();
  };
  const trustedHostNames = (target: string, port: number): string[] => hosts.filter((host) => host.address === target && host.port === port).map((host) => host.name);
  const settingsLabel: Record<SettingsTab, string> = {
    security: labels.settingsSecurity, keys: labels.settingsHostKeys, credentials: labels.settingsCredentials, groups: labels.settingsGroups, desktop: labels.settingsDesktop, shortcuts: labels.settingsShortcuts,
  };
  const messages = error || notice ? <div className="message-stack">
    {error && <p className="inline-error" role="alert"><AlertIcon size={14} /><span>{error}</span>
      <button className="icon-button" type="button" aria-label={labels.dismiss} title={labels.dismiss} onClick={() => setError(null)}><CloseIcon size={12} /></button></p>}
    {notice && <p className="inline-notice" role="status"><CheckIcon size={14} /><span>{notice}</span>
      <button className="icon-button" type="button" aria-label={labels.dismiss} title={labels.dismiss} onClick={() => setNotice(null)}><CloseIcon size={12} /></button></p>}
  </div> : null;
  const sheetKeyDown = (event: KeyboardEvent<HTMLElement>): void => {
    // Dialogs portaled out of a sheet (the snippet editor, confirmations) still
    // bubble through React; they own their own Tab and Escape.
    if (!(event.target instanceof Node) || !event.currentTarget.contains(event.target)) return;
    if (event.key === "Tab") {
      // Keep keyboard focus inside the sheet while the workspace behind it is inert.
      const focusable = [...event.currentTarget.querySelectorAll<HTMLElement>("button:not([disabled]), input:not([disabled]), select:not([disabled]), summary, [tabindex='0']")]
        .filter((element) => element.offsetParent !== null || element === document.activeElement);
      const first = focusable[0], last = focusable.at(-1);
      if (first && last && event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
      else if (first && last && !event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
      return;
    }
    if (event.key !== "Escape") return;
    event.stopPropagation();
    if (sheet === "host") cancelHostEdit(); else closeSheet();
  };
  const settingsTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, current: SettingsTab): void => {
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (!step && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const index = settingsTabs.indexOf(current);
    const next = event.key === "Home" ? settingsTabs[0] : event.key === "End" ? settingsTabs[settingsTabs.length - 1]
      : settingsTabs[(index + step + settingsTabs.length) % settingsTabs.length];
    setSettingsTab(next);
    requestAnimationFrame(() => document.getElementById(`settings-tab-${next}`)?.focus());
  };
  type WorkspaceLabel = "newTerminalAction" | "closeTerminalAction" | "splitRightAction" | "splitDownAction"
    | "toggleBroadcastAction" | "toggleSidebarAction" | "focusFilesAction" | "focusTransfersAction" | "lockVaultAction";
  const workspaceActions: Array<[WorkspaceCommand, WorkspaceLabel, string | null]> = [
    ["new_terminal", "newTerminalAction", "Ctrl+Shift+T"],
    ...(activeEntry ? [
      ["close_terminal", "closeTerminalAction", "Ctrl+Shift+W"],
      ["split_right", "splitRightAction", "Alt+Shift+="],
      ["split_down", "splitDownAction", "Alt+Shift+-"],
      ["toggle_broadcast", "toggleBroadcastAction", null],
    ] as Array<[WorkspaceCommand, WorkspaceLabel, string | null]> : []),
    ...(vault.unlocked ? [["toggle_sidebar", "toggleSidebarAction", "Ctrl+Shift+B"]] as Array<[WorkspaceCommand, WorkspaceLabel, string | null]> : []),
    ...(activeEntry?.launch.kind === "ssh" ? [["focus_files", "focusFilesAction", "Ctrl+Shift+E"]] as Array<[WorkspaceCommand, WorkspaceLabel, string | null]> : []),
    ...(isTauriAvailable() ? [["focus_transfers", "focusTransfersAction", "Ctrl+Shift+J"]] as Array<[WorkspaceCommand, WorkspaceLabel, string | null]> : []),
    ...(vault.unlocked ? [["lock_vault", "lockVaultAction", null]] as Array<[WorkspaceCommand, WorkspaceLabel, string | null]> : []),
  ];
  const snippetGroupNames = new Map(snippetGroups.map((group) => [group.id, group.name]));
  const paletteActions: PaletteAction[] = [
    ...hosts.map((host) => ({
      key: `host:${host.id}`,
      action: { id: "connect_host", labelKey: "connectHost", keywordKeys: ["host", "connect"], hostId: host.id } as CommandAction,
      label: host.name,
      detail: hostAddressLabel(host),
      detailMono: true,
      production: productionHosts.has(host.id),
      keywords: [labels.connectHost, labels.hosts, host.name, hostAddressLabel(host), ...host.tags, ...(productionHosts.has(host.id) ? [labels.production] : [])],
    })),
    ...snippets.map((snippet) => ({
      key: `snippet:${snippet.id}`,
      action: { id: "run_snippet", labelKey: "runSnippet", keywordKeys: ["snippet", "run"], snippetId: snippet.id } as CommandAction,
      label: snippet.name,
      ...(snippet.group_id && snippetGroupNames.has(snippet.group_id) ? { detail: snippetGroupNames.get(snippet.group_id) } : {}),
      keywords: [labels.runSnippet, labels.snippets, snippet.name, ...(snippet.group_id ? [snippetGroupNames.get(snippet.group_id) ?? ""] : [])],
    })),
    ...shellProfiles.map((profile) => ({
      key: `shell:${profile.id}`,
      action: { id: "open_shell", labelKey: "newShellAction", keywordKeys: ["shell", "terminal"], shellId: profile.id } as CommandAction,
      label: labels.newShellAction(profile.name),
      keywords: [labels.newTerminal, labels.paletteSections.shells, profile.name, ...(profile.is_wsl ? ["wsl", "linux"] : [])],
    })),
    ...([{ id: "graphite", name: labels.themeGraphite }, { id: "light", name: labels.themeLight }, ...themes] as const).map(({ id, name }) => ({
      key: `theme:${id}`,
      action: { id: "set_theme", labelKey: "changeTheme", keywordKeys: ["theme", "appearance"], theme: id } as CommandAction,
      label: labels.themeAction(name),
      detail: labels.changeThemeScope,
      keywords: [labels.changeTheme, name, id],
    })),
    ...(vault.unlocked ? [{
      key: "panel:snippets",
      action: { id: "toggle_panel", labelKey: "showSnippets", keywordKeys: ["snippet", "panel"], panel: "snippets" } as CommandAction,
      label: labels.openSnippets,
      keywords: [labels.snippets, labels.showSnippets],
    }, {
      key: "panel:forwarding",
      action: { id: "toggle_panel", labelKey: "tunnels", keywordKeys: ["panel"], panel: "forwarding" } as CommandAction,
      label: labels.openTunnels,
      keywords: [labels.tunnels, labels.tunnelLocal, labels.tunnelRemote, labels.tunnelSocks],
    }, {
      key: "settings",
      action: { id: "open_settings", labelKey: "openSettings", keywordKeys: ["settings", "preferences"] } as CommandAction,
      label: labels.openSettings,
      shortcut: "Ctrl+,",
      keywords: [labels.openSettings, labels.settings],
    }] : []),
    ...workspaceActions.map(([command, labelKey, shortcut]) => ({
      key: `workspace:${command}`,
      action: { id: "workspace", labelKey, keywordKeys: ["workspace"], command } as CommandAction,
      label: labels[labelKey],
      ...(shortcut ? { shortcut } : {}),
      keywords: [labels.workspaceAction, labels[labelKey]],
    })),
  ];
  // Same code paths as the shortcuts and toolbar keys, so the palette never
  // gains a second, divergent way to split, close, or broadcast.
  const runWorkspaceCommand = (command: WorkspaceCommand): void => {
    const workspace = useWorkspace.getState();
    const tab = workspace.tabs.find((candidate) => candidate.id === workspace.activeTab);
    let focusTerminal = true;
    if (command === "new_terminal") addTerminal();
    else if (command === "close_terminal") { if (tab) closeTerminal(tab.id); }
    else if (command === "split_right" || command === "split_down") { if (tab) workspace.splitPane(tab.id, tab.focusedPaneId, command === "split_right" ? "horizontal" : "vertical"); }
    else if (command === "toggle_broadcast") { if (tab) workspace.setBroadcast(tab.id, !tab.broadcastEnabled); }
    else if (command === "toggle_sidebar") toggleSidebar();
    else if (command === "lock_vault") { void lockVault(); focusTerminal = false; }
    // Focus moves wait a frame, until the palette's focus trap is gone.
    const moveFocus = command === "focus_files" ? focusFiles : command === "focus_transfers" ? () => focusQueues(true) : null;
    if (moveFocus) requestAnimationFrame(() => { if (!moveFocus()) setFocusRequest((request) => request + 1); });
    else if (focusTerminal) requestAnimationFrame(() => setFocusRequest((request) => request + 1));
  };
  const executePaletteAction = (action: CommandAction): void => {
    // Opened from a sheet, an action that works in the workspace closes the
    // sheet first; actions that open a sheet replace it.
    const opensSheet = action.id === "run_snippet" || action.id === "toggle_panel" || action.id === "open_settings";
    if (sheet && !opensSheet) closeSheet();
    if (action.id === "open_shell") {
      const profile = shellProfiles.find((candidate) => candidate.id === action.shellId);
      addTerminal({ shell_id: action.shellId }, profile?.name);
      requestAnimationFrame(() => setFocusRequest((request) => request + 1));
    } else if (action.id === "connect_host") {
      const host = hosts.find((candidate) => candidate.id === action.hostId);
      if (host) openHost(host);
    } else if (action.id === "run_snippet") {
      setSnippetRequest({ requestId: crypto.randomUUID(), snippetId: action.snippetId });
      openSheet("snippets");
    } else if (action.id === "set_theme") {
      const workspace = useWorkspace.getState();
      for (const tab of workspace.tabs) for (const pane of paneLeaves(tab.root)) {
        workspace.setPaneTheme(tab.id, pane.paneId, action.theme);
      }
      setNotice(labels.themeChanged(action.theme === "light" ? labels.themeLight : themes.find(({ id }) => id === action.theme)?.name ?? labels.themeGraphite));
      requestAnimationFrame(() => setFocusRequest((request) => request + 1));
    } else if (action.id === "toggle_panel" && action.panel === "snippets") {
      openSheet("snippets");
    } else if (action.id === "toggle_panel" && action.panel === "forwarding") {
      openSheet("tunnels");
    } else if (action.id === "workspace") {
      runWorkspaceCommand(action.command);
    } else if (action.id === "open_settings") {
      openSheet("settings");
    }
  };
  const terminalTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, id: string): void => {
    const index = tabs.findIndex((tab) => tab.id === id);
    if (event.altKey && event.shiftKey && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
      event.preventDefault();
      const target = tabs[index + (event.key === "ArrowRight" ? 1 : -1)];
      if (target) moveTab(tabs[index].id, target.id);
      requestAnimationFrame(() => document.getElementById(id + "-tab")?.focus());
      return;
    }
    if (event.key === "Delete") { event.preventDefault(); closeTerminal(tabs[index].id); return; }
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (!step && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const next = event.key === "Home" ? tabs[0] : event.key === "End" ? tabs[tabs.length - 1] : tabs[(index + step + tabs.length) % tabs.length];
    setActiveTab(next.id);
    requestAnimationFrame(() => document.getElementById(next.id + "-tab")?.focus());
  };
  const fieldError = (field: HostField): ReactElement | null => fieldErrors.has(field)
    ? <span className="field-error" id={`host-${field}-error`}>{field === "port" ? labels.portRangeError : labels.fieldRequired}</span> : null;
  const invalid = (field: HostField) => fieldErrors.has(field)
    ? { "aria-invalid": true, "aria-describedby": `host-${field}-error` } as const : {};
  const clearFieldError = (field: HostField): void => {
    if (fieldErrors.has(field)) setFieldErrors((current) => { const next = new Set(current); next.delete(field); return next; });
  };
  const advancedSession = form.workingDirectory !== "" || form.startupCommand !== "" || form.encoding !== "utf-8";
  const inheritedProduction = form.production ? null
    : productionSource({ production: false, tags: form.tags.split(",").map((tag) => tag.trim()).filter(Boolean), group_id: form.groupId || null }, groupsById);

  return (
    <main className="application" aria-label={labels.title}>
      <LaunchRequests onRequest={processLaunchRequest} />
      <UpdateCenter locale={locale} />
      <HostChoiceDialog open={pendingHostChoice !== null} target={pendingHostChoice?.display ?? ""}
        hosts={pendingHostChoice?.hosts ?? []} title={labels.sshChooseHostTitle}
        body={labels.sshChooseHostBody(pendingHostChoice?.hosts.length ?? 0)} cancel={labels.cancelEdit}
        onChoose={chooseLaunchHost} onCancel={cancelLaunchHostChoice} />
      <Titlebar labels={labels} context={titleContext} mono={!!activeHost} badge={activeHost && productionHosts.has(activeHost.id) ? labels.production : undefined} />
      <div className={"app-layout" + (sidebarCollapsed ? " sidebar-hidden" : "")}>
        {narrow && sidebarOpen && <div className="sidebar-scrim" aria-hidden="true" onClick={() => setSidebarOpen(false)} />}
        <aside className={"sidebar" + (sidebarOpen ? " sidebar-open" : "")} aria-label={labels.hostSearchLabel}>
          <header className="sidebar-heading">
            <h2>{labels.hosts}</h2>
            <div className="sidebar-heading-actions">
              {vault.unlocked && <button className="icon-button" type="button" aria-label={labels.addHost} title={labels.addHost} onClick={() => addHost()}><PlusIcon /></button>}
              {narrow && <button className="icon-button sidebar-close" type="button" aria-label={labels.closeSidebar} title={labels.closeSidebar} onClick={() => setSidebarOpen(false)}><CloseIcon /></button>}
            </div>
          </header>
          {vault.unlocked ? (
            <>
              <div className="sidebar-search">
                <SearchIcon size={14} />
                <input ref={searchRef} aria-label={labels.searchHosts} placeholder={labels.searchHosts} title={labels.shortcut(labels.searchHosts, "Ctrl+Shift+K")}
                  value={search} onChange={(event) => setSearch(event.currentTarget.value)}
                  onKeyDown={(event) => {
                    if (event.key === "ArrowDown") { event.preventDefault(); document.querySelector<HTMLButtonElement>(".sidebar .host-open")?.focus(); return; }
                    if (event.key === "Escape") {
                      event.preventDefault();
                      if (search) setSearch(""); else setFocusRequest((request) => request + 1);
                      return;
                    }
                    if (event.key !== "Enter") return;
                    const first = visibleHosts[0];
                    if (first) openHost(first, event.ctrlKey);
                    else if (quickHost) addHost({ name: quickHost.address, address: quickHost.address, port: quickHost.port, username: quickHost.username });
                  }} />
              </div>
              <section className="host-list" aria-label={labels.hostSearchLabel}>
                {visibleHosts.length === 0 ? (hosts.length === 0
                  ? <div className="sidebar-empty"><p>{labels.noHosts}</p><button className="primary-action" type="button" onClick={() => addHost()}><PlusIcon size={14} />{labels.addHost}</button></div>
                  : <div className="sidebar-empty"><p>{labels.noHostMatches(search.trim())}</p>
                    {quickHost && <button type="button" onClick={() => addHost({ name: quickHost.address, address: quickHost.address, port: quickHost.port, username: quickHost.username })}>
                      <PlusIcon size={14} />{labels.quickAddHost(search.trim())}</button>}
                  </div>)
                  : hostSections.map((section) => <div key={`${section.favorites}:${section.label}`} role="group" aria-label={section.label}>
                    <h3 className="host-group-label">{section.favorites ? <StarIcon size={11} /> : null}{section.label}<span className="count">{section.hosts.length}</span></h3>
                    {section.hosts.map((host) => {
                      const signal = hostSignals.get(host.id);
                      const production = productionHosts.has(host.id);
                      return <div className={"host-row" + (signal === "live" ? " is-live" : "")} data-prod={production || undefined} key={host.id} draggable onDragStart={(event) => {
                        event.dataTransfer.effectAllowed = "move";
                        event.dataTransfer.setData("application/x-capstan-host", host.id);
                      }}>
                        <button className="host-open" type="button" title={labels.openHostHint(host.name)} tabIndex={host.id === tabbableHost ? 0 : -1}
                          onFocus={() => setRovingHost(host.id)} onKeyDown={(event) => hostRowKeyDown(event, host)}
                          onClick={(event) => openHost(host, event.ctrlKey || event.shiftKey)}
                          onAuxClick={(event) => { if (event.button === 1) { event.preventDefault(); openHost(host, true); } }}>
                          <span className="host-key" aria-hidden="true">{hostMonogram(host.name)}{signal && <span className="led" data-state={signal} />}</span>
                          <span className="host-text">
                            <span className="host-row-name"><span>{host.name}</span>{production && <span className="prod-chip" aria-hidden="true">{labels.production}</span>}</span>
                            <span className="host-row-address">{host.username}@{host.address}{host.port === 22 ? "" : `:${host.port}`}{section.favorites && host.group_id && !production ? <span className="host-row-group"> · {groupNames.get(host.group_id) ?? labels.noGroup}</span> : null}</span>
                          </span>
                          {(production || signal) && <span className="sr-only">{[production ? labels.production : "", signal ? labels.tabStatus[signal] : ""].filter(Boolean).join(", ")}</span>}
                        </button>
                        {/* Out of the Tab order; F2 on the focused row is the keyboard path. */}
                        <button className="host-edit" type="button" tabIndex={-1} aria-label={labels.editHost + ": " + host.name} aria-keyshortcuts="F2" title={labels.shortcut(labels.editHost, "F2")} onClick={() => editHost(host)}><PencilIcon size={14} /></button>
                      </div>;
                    })}
                  </div>)}
              </section>
            </>
          ) : (
            <section className="vault-locked">
              <span className="vault-locked-key" aria-hidden="true"><LockIcon size={20} /></span>
              <h2>{labels.vaultLocked}</h2>
              <p className="quiet-copy">{labels.localOnlyBeforeUnlock}</p>
              {isTauriAvailable() ? (
                <div className="vault-actions">
                  {vault.exists
                    ? <button ref={masterUnlockRef} className="primary-action" type="button" disabled={vaultBusy} onClick={() => { void runVaultAction("unlock_vault"); }}>{labels.unlockVault}</button>
                    : <button className="primary-action" type="button" disabled={vaultBusy} onClick={() => { void runVaultAction("initialize_vault"); }}>{labels.initializeVault}</button>}
                  {vault.exists && hello?.enabled && <button type="button" disabled={vaultBusy || !hello.available} onClick={() => { void runHelloAction("unlock_vault_hello"); }}>{labels.hello.unlock}</button>}
                  {vault.exists && <button className="ghost-button" type="button" disabled={vaultBusy} onClick={() => { void findRecoveryCandidates(); }}>{labels.findRecoveryFiles}</button>}
                </div>
              ) : <p className="quiet-copy">{labels.vaultUnavailable}</p>}
              {vaultBusy && <p className="quiet-copy" role="status">{labels.hello.busy}</p>}
              {hello?.enabled && !hello.available && <p className="quiet-copy">{labels.hello.unavailable}</p>}
              {recoveryCandidates.length > 0 && <section className="recovery-candidates" aria-label={labels.recoveryCandidatesTitle}>
                <p className="quiet-copy">{labels.recoverySelectHint}</p>
                <ul>
                  {recoveryCandidates.map((candidate) => <li key={candidate.token}>
                    <span>{candidate.kind === "pending" ? labels.recoveryPending : labels.recoveryBackup} · {labels.recoveryRevision(candidate.revision)}</span>
                    <button type="button" disabled={restoringRecovery !== null} onClick={() => { void restoreRecoveryCandidate(candidate); }}>
                      {restoringRecovery === candidate.token ? labels.restoringRecovery : labels.restoreRecovery}
                    </button>
                  </li>)}
                </ul>
              </section>}
            </section>
          )}
          {!sheet && sidebarVisible && messages}
          {vault.unlocked && <footer className="sidebar-footer">
            <span className="vault-state" title={labels.autoLockStatus(preferences.auto_lock_minutes)}>
              <UnlockIcon size={14} />
              <span>{labels.vaultReady}</span>
            </span>
            <div className="sidebar-footer-actions">
              <button className="icon-button" type="button" aria-label={labels.commandPalette} title={labels.shortcut(labels.commandPalette, "Ctrl+Shift+P")}
                aria-keyshortcuts="Control+Shift+P" onClick={() => { setCommandPaletteOpen(true); }}><CommandIcon size={14} /></button>
              <button className="icon-button" type="button" aria-label={labels.settings} title={labels.shortcut(labels.settings, "Ctrl+,")} aria-keyshortcuts="Control+Comma" aria-expanded={sheet === "settings"}
                onClick={() => { if (sheet === "settings") closeSheet(); else openSheet("settings"); }}><SettingsIcon /></button>
              <button className="icon-button" type="button" aria-label={labels.lockVault} title={labels.lockVault} onClick={() => { void lockVault(); }}><LockIcon /></button>
            </div>
          </footer>}
        </aside>
        {sheet && <div className="sheet-scrim" aria-hidden="true" onClick={() => { if (sheet === "host") cancelHostEdit(); else closeSheet(); }} />}
        {sheet === "snippets" && <SnippetManager
          panelRef={sheetRef}
          onKeyDown={sheetKeyDown}
          labels={labels}
          groups={snippetGroups}
          snippets={snippets}
          search={snippetSearch}
          onSearch={setSnippetSearch}
          onSaveGroup={saveSnippetGroup}
          onDeleteGroup={deleteSnippetGroup}
          onSaveSnippet={saveSnippet}
          onDeleteSnippet={deleteSnippet}
          onRun={runSnippetInput}
          broadcastTargetsAvailable={(activeEntry?.broadcastPaneIds.length ?? 0) > 0}
          runRequest={snippetRequest}
          onRequestConsumed={(requestId) => setSnippetRequest((current) => current?.requestId === requestId ? undefined : current)}
          messages={messages}
          onClose={closeSheet}
        />}
        {sheet === "tunnels" && <TunnelManager labels={labels} confirm={confirm}
          hosts={hosts.map((host) => ({ ...host, production: isProductionHost(host, groupsById) }))}
          revision={String(vault.revision ?? 0)} onRefreshVault={refreshVault} onConnectHost={(host) => { openHost(host); closeSheet(); }}
          onClose={closeSheet} panelRef={sheetRef} onKeyDown={sheetKeyDown} />}
        {sheet === "host" && <aside ref={sheetRef} className="sheet" role="dialog" aria-labelledby="sheet-title" tabIndex={-1} onKeyDown={sheetKeyDown}>
          <form className="host-editor" noValidate onSubmit={(event) => { void saveHost(event); }}>
            <header className="sheet-header">
              <h2 id="sheet-title"><ServerIcon size={14} />{editingHost ? labels.editHost : labels.addHost}</h2>
              <button className="icon-button" type="button" aria-label={labels.closePanel} title={labels.closePanel} onClick={cancelHostEdit}><CloseIcon /></button>
            </header>
            {messages}
            <div className="sheet-body">
              <section className="form-section">
                <h3>{labels.hostSectionConnection}</h3>
                <label>{labels.hostName}<input value={form.name} onChange={(event) => { clearFieldError("name"); setForm({ ...form, name: event.currentTarget.value }); }} required maxLength={256} {...invalid("name")} />{fieldError("name")}</label>
                <div className="form-pair address">
                  <label>{labels.hostAddress}<input className="mono" value={form.address} onChange={(event) => { clearFieldError("address"); setForm({ ...form, address: event.currentTarget.value }); }} required maxLength={253} spellCheck={false} {...invalid("address")} />{fieldError("address")}</label>
                  <label>{labels.hostPort}<input className="mono" inputMode="numeric" value={form.port} onChange={(event) => { clearFieldError("port"); setForm({ ...form, port: event.currentTarget.value }); }} required {...invalid("port")} />{fieldError("port")}</label>
                </div>
                <label>{labels.hostUsername}<input className="mono" value={form.username} onChange={(event) => { clearFieldError("username"); setForm({ ...form, username: event.currentTarget.value }); }} required maxLength={256} spellCheck={false} {...invalid("username")} />{fieldError("username")}</label>
                <label className="check-row production-row"><input type="checkbox" checked={form.production} onChange={(event) => setForm({ ...form, production: event.currentTarget.checked })} aria-describedby="host-production-hint" />{labels.productionHost}</label>
                <p className="quiet-copy" id="host-production-hint">{inheritedProduction?.kind === "group" ? labels.productionFromGroup(inheritedProduction.name)
                  : inheritedProduction?.kind === "tag" ? labels.productionFromTag(inheritedProduction.name) : labels.productionHostHint}</p>
              </section>
              <section className="form-section">
                <h3>{labels.hostSectionAuth}</h3>
                <label>{labels.credentialSelect}
                  <select value={form.credentialId} onChange={(event) => {
                    const credentialId = event.currentTarget.value as CredentialId;
                    const credential = credentials.find((candidate) => candidate.id === credentialId);
                    clearFieldError("credentialId");
                    setForm({ ...form, credentialId, agentForwarding: credential?.kind === "agent" && form.agentForwarding });
                  }} required {...invalid("credentialId")}>
                    <option value="">{labels.credentialNone}</option>
                    {credentials.map((credential) => <option value={credential.id} key={credential.id}>{credential.name} · {credentialKindLabel(credential.kind)}</option>)}
                  </select>
                  {fieldError("credentialId")}
                </label>
                <details className="credential-create" open={credentials.length === 0}>
                  <summary><ChevronRightIcon size={14} />{labels.newCredential}</summary>
                  <div className="credential-create-body">
                    <input aria-label={labels.credentialName} placeholder={labels.credentialName} value={credentialName} onChange={(event) => setCredentialName(event.currentTarget.value)} maxLength={256} />
                    <div className="credential-create-row">
                      <button type="button" disabled={!credentialName.trim()} onClick={() => { void createCredential(); }}>{labels.createCredential}</button>
                      <button type="button" disabled={!credentialName.trim()} onClick={() => { void importPrivateKey(); }}>{labels.importPrivateKey}</button>
                      <button type="button" disabled={!credentialName.trim()} onClick={() => { void linkPrivateKeyFile(); }}>{labels.linkPrivateKeyFile}</button>
                    </div>
                    <div className="agent-row">
                      <label>{labels.sshAgent}
                        <select value={agentProvider} onChange={(event) => {
                          setAgentProvider(event.currentTarget.value as AgentProvider);
                          setAgentIdentities([]);
                          setAgentFingerprint("");
                        }}>
                          <option value="open_ssh">{labels.openSshAgent}</option>
                          <option value="pageant">Pageant</option>
                        </select>
                      </label>
                      <button type="button" onClick={() => { void loadAgentIdentities(); }}>{labels.loadAgentIdentities}</button>
                    </div>
                    <div className="agent-row">
                      <label>{labels.agentIdentity}
                        <select value={agentFingerprint} onChange={(event) => setAgentFingerprint(event.currentTarget.value)}>
                          <option value="">{labels.selectAgentIdentity}</option>
                          {agentIdentities.map((identity) => <option value={identity.fingerprint} key={identity.fingerprint}>
                            {identity.algorithm} · {identity.fingerprint}
                          </option>)}
                        </select>
                      </label>
                      <button type="button" disabled={!credentialName.trim() || !agentFingerprint} onClick={() => { void addAgentCredential(); }}>{labels.addAgentCredential}</button>
                    </div>
                  </div>
                </details>
                <label className="check-row"><input
                  type="checkbox"
                  checked={form.agentForwarding}
                  disabled={credentials.find((credential) => credential.id === form.credentialId)?.kind !== "agent"}
                  onChange={(event) => setForm({ ...form, agentForwarding: event.currentTarget.checked })}
                />{labels.agentForwarding}</label>
                <p className="quiet-copy">{labels.credentialDialogHint}</p>
              </section>
              <section className="form-section">
                <h3>{labels.hostSectionSession}</h3>
                <div className="form-pair even">
                  <label>{labels.hostGroup}
                    <select value={form.groupId} onChange={(event) => setForm({ ...form, groupId: event.currentTarget.value as GroupId | "" })}>
                      <option value="">{labels.noGroup}</option>
                      {groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}
                    </select>
                  </label>
                  <label>{labels.terminalTheme}<select value={form.theme} onChange={(event) => setForm({ ...form, theme: event.currentTarget.value })}>
                    <option value="graphite">{labels.themeGraphite}</option><option value="light">{labels.themeLight}</option>
                    {themes.map((theme) => <option key={theme.id} value={theme.id}>{theme.name}</option>)}
                    {form.theme !== "graphite" && form.theme !== "light" && !themes.some(({ id }) => id === form.theme)
                      && <option value={form.theme}>{labels.missingTheme}</option>}
                  </select></label>
                </div>
                <label>{labels.hostTags}<input value={form.tags} onChange={(event) => setForm({ ...form, tags: event.currentTarget.value })} /></label>
                <label className="check-row"><input type="checkbox" checked={form.favorite} onChange={(event) => setForm({ ...form, favorite: event.currentTarget.checked })} />{labels.favoriteHost}</label>
                <details className="form-advanced" open={advancedSession}>
                  <summary><ChevronRightIcon size={14} />{labels.advancedSession}</summary>
                  <div className="form-advanced-body">
                    <label>{labels.workingDirectory}<input className="mono" value={form.workingDirectory} onChange={(event) => setForm({ ...form, workingDirectory: event.currentTarget.value })} spellCheck={false} /></label>
                    <label>{labels.startupCommand}<input className="mono" value={form.startupCommand} onChange={(event) => setForm({ ...form, startupCommand: event.currentTarget.value })} spellCheck={false} /></label>
                    <label>{labels.terminalEncoding}<select value={form.encoding} onChange={(event) => setForm({ ...form, encoding: event.currentTarget.value })}><option value="utf-8">UTF-8</option></select></label>
                  </div>
                </details>
              </section>
            </div>
            <footer className="sheet-footer form-actions">
              {editingHost && <button className="danger-action" type="button" onClick={() => { void deleteHost(editingHost); }}>{labels.deleteHost}</button>}
              {credentials.length === 0 && <span className="footer-hint" id="host-save-hint">{labels.saveHostNeedsCredential}</span>}
              <span className="spacer" />
              <button type="submit" disabled={credentials.length === 0} aria-describedby={credentials.length === 0 ? "host-save-hint" : undefined}>{labels.saveHost}</button>
              <button className="ghost-button" type="button" onClick={cancelHostEdit}>{labels.cancelEdit}</button>
            </footer>
          </form>
        </aside>}
        {sheet === "settings" && <aside ref={sheetRef} className="sheet" role="dialog" aria-labelledby="sheet-title" tabIndex={-1} onKeyDown={sheetKeyDown}>
          <header className="sheet-header">
            <h2 id="sheet-title"><SettingsIcon size={14} />{labels.settings}</h2>
            <button className="icon-button" type="button" aria-label={labels.closePanel} title={labels.closePanel} onClick={closeSheet}><CloseIcon /></button>
          </header>
          <div className="segmented settings-tabs" role="tablist" aria-label={labels.settings}>
            {settingsTabs.map((tab) => <button key={tab} id={`settings-tab-${tab}`} type="button" role="tab" aria-selected={settingsTab === tab}
              aria-controls="settings-panel" tabIndex={settingsTab === tab ? 0 : -1} onClick={() => setSettingsTab(tab)}
              onKeyDown={(event) => settingsTabKeyDown(event, tab)}>{settingsLabel[tab]}</button>)}
          </div>
          {messages}
          <div className="sheet-body" id="settings-panel" role="tabpanel" aria-labelledby={`settings-tab-${settingsTab}`}>
            {settingsTab === "desktop" && <><ThemeSettings locale={locale} /><UpdateSettings locale={locale} /></>}
            {settingsTab === "security" && <section className="settings-section vault-settings">
              <h3>{labels.vaultSettings}</h3>
              <div className="settings-group" aria-busy={vaultBusy}>
                <h4>{labels.hello.title}</h4>
                <p className="quiet-copy">{labels.hello.hint}</p>
                {(vaultBusy || !hello || !hello.available || hello.enabled) && <p className="quiet-copy" role="status">{vaultBusy ? labels.hello.busy : !hello ? labels.hello.checking : !hello.available ? labels.hello.unavailable : labels.hello.enabled}</p>}
                <div className="settings-actions">
                  {hello?.enabled
                    ? <button type="button" disabled={vaultBusy} onClick={() => { void runHelloAction("disable_vault_hello"); }}>{labels.hello.disable}</button>
                    : <button type="button" disabled={vaultBusy || !hello?.available} onClick={() => { void runHelloAction("enable_vault_hello"); }}>{labels.hello.enable}</button>}
                </div>
              </div>
              <div className="settings-group">
                <label>{labels.autoLockTimeout}<input
                  type="number" min="1" max="1440" inputMode="numeric"
                  value={preferences.auto_lock_minutes ?? ""}
                  onChange={(event) => {
                    const value = event.currentTarget.value;
                    setPreferences((current) => ({
                      ...current,
                      auto_lock_minutes: value === "" ? null : Number(value),
                    }));
                  }}
                /></label>
                <label className="check-row"><input type="checkbox" checked={preferences.disconnect_on_lock} onChange={(event) => {
                  const checked = event.currentTarget.checked;
                  setPreferences((current) => ({ ...current, disconnect_on_lock: checked }));
                }} />{labels.disconnectOnLock}</label>
                <div className="settings-actions">
                  <button className="primary-action" type="button" onClick={() => { void saveVaultSettings(); }}>{labels.saveVaultSettings}</button>
                </div>
              </div>
              <div className="settings-group">
                <h4>{labels.vaultBackup}</h4>
                <div className="settings-actions">
                  <button type="button" onClick={() => { void changeMasterPassword(); }}><LockIcon size={14} />{labels.changeMasterPassword}</button>
                  <button type="button" onClick={() => { void exportVaultBackup(); }}><DownloadIcon size={14} />{labels.exportVaultBackup}</button>
                  <button type="button" onClick={() => { void previewVaultBackup(); }}><UploadIcon size={14} />{labels.previewVaultBackup}</button>
                </div>
              </div>
            </section>}
            {settingsTab === "keys" && <section className="settings-section trust-manager">
              <h3>{labels.trustedHostKeys}</h3>
              <p className="quiet-copy">{labels.trustedHostKeyPolicy}</p>
              {trustedHostKeys.length === 0 ? <p className="quiet-copy">{labels.noTrustedHostKeys}</p> : <ul className="item-list">
                {trustedHostKeys.map((trusted) => {
                  const names = trustedHostNames(trusted.target, trusted.port);
                  return <li key={`${trusted.target}:${trusted.port}`}>
                    <div className="item-main">
                      <strong>{names.length ? names.join(", ") : `${trusted.target}:${trusted.port}`}</strong>
                      {names.length > 0 && <span className="mono">{trusted.target}:{trusted.port}</span>}
                      <span className="mono">{trusted.algorithm}</span>
                      <span className="mono fingerprint-line">{trusted.fingerprint}</span>
                      <span>{labels.firstTrustedAt(new Date(trusted.first_trusted_at * 1000).toLocaleString(locale))}</span>
                    </div>
                    <button className="danger-action" type="button" aria-label={`${labels.forgetTrustedHostKey}: ${trusted.target}:${trusted.port}`}
                      onClick={() => { void forgetTrustedHostKey(trusted); }}>{labels.forgetTrustedHostKey}</button>
                  </li>;
                })}
              </ul>}
            </section>}
            {settingsTab === "credentials" && <section className="settings-section credential-manager">
              <h3>{labels.manageCredentials}</h3>
              <p className="quiet-copy">{labels.credentialManagerHint}</p>
              {credentials.length === 0 ? <p className="quiet-copy">{labels.noCredentials}</p> : <ul className="item-list credential-list">
                {credentials.map((credential) => <li key={credential.id}>
                  {renamingCredentialId === credential.id ? <form onSubmit={(event) => { event.preventDefault(); void renameCredential(credential); }}>
                    <input aria-label={labels.credentialName} value={credentialRename} onChange={(event) => setCredentialRename(event.currentTarget.value)} maxLength={256} />
                    <button className="primary-action" type="submit">{labels.saveCredentialName}</button>
                    <button className="ghost-button" type="button" onClick={() => { setRenamingCredentialId(null); setCredentialRename(""); }}>{labels.cancelCredentialEdit}</button>
                  </form> : <>
                    <div className="item-main"><strong>{credential.name}</strong><span>{credentialKindLabel(credential.kind)}</span></div>
                    <div className="credential-actions">
                      <button type="button" onClick={() => beginRenameCredential(credential)}>{labels.renameCredential}</button>
                      {credential.kind === "password" && <button type="button" onClick={() => { void updatePasswordCredential(credential); }}>{labels.updateCredentialPassword}</button>}
                      <button className="danger-action" type="button" onClick={() => { void deleteCredential(credential); }}>{labels.deleteCredential}</button>
                    </div>
                  </>}
                </li>)}
              </ul>}
            </section>}
            {settingsTab === "groups" && <section className="settings-section group-editor">
              <h3>{editingGroup ? labels.editGroup : labels.manageGroups}</h3>
              <form className="settings-group" onSubmit={(event) => { void saveGroup(event); }}>
                <div className="form-pair even">
                  <label>{labels.groupName}<input value={groupName} onChange={(event) => setGroupName(event.currentTarget.value)} required maxLength={256} /></label>
                  <label>{labels.groupParent}
                    <select value={groupParentId} onChange={(event) => setGroupParentId(event.currentTarget.value as GroupId | "")}>
                      <option value="">{labels.noGroup}</option>
                      {groups.filter((group) => group.id !== editingGroup?.id).map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}
                    </select>
                  </label>
                </div>
                <label className="check-row"><input type="checkbox" checked={groupProduction} onChange={(event) => setGroupProduction(event.currentTarget.checked)} aria-describedby="group-production-hint" />{labels.productionGroup}</label>
                <p className="quiet-copy" id="group-production-hint">{labels.productionGroupHint}</p>
                <div className="form-actions">
                  {editingGroup && <button className="danger-action" type="button" onClick={() => { void deleteGroup(editingGroup); }}>{labels.deleteGroup}</button>}
                  <span className="spacer" />
                  <button type="submit">{labels.saveGroup}</button>
                  {editingGroup && <button className="ghost-button" type="button" onClick={() => { setEditingGroup(null); setGroupName(""); setGroupParentId(""); setGroupProduction(false); }}>{labels.cancelEdit}</button>}
                </div>
              </form>
              <p className="quiet-copy">{labels.groupDropTargets}</p>
              <ul className="item-list group-list" aria-label={labels.settingsGroups}>
                {groups.map((group) => <li key={group.id} draggable className={dropGroup === group.id ? "is-drop-target" : undefined}
                  style={{ marginInlineStart: `${groupDepth(group) * 16}px` }}
                  onDragStart={(event) => {
                    event.dataTransfer.effectAllowed = "move";
                    event.dataTransfer.setData("application/x-capstan-group", group.id);
                  }}
                  onDragOver={(event) => { event.preventDefault(); setDropGroup(group.id); }}
                  onDragLeave={() => setDropGroup((current) => current === group.id ? null : current)}
                  onDrop={(event) => { setDropGroup(null); dropOnGroup(event, group); }}>
                  <div className="item-main"><strong><FolderIcon size={14} />{group.name}{group.production && <span className="prod-chip">{labels.production}</span>}</strong></div>
                  <button type="button" onClick={() => editGroup(group)}>{labels.editGroup}</button>
                </li>)}
              </ul>
            </section>}
            {settingsTab === "shortcuts" && <section className="settings-section shortcut-reference">
              <h3>{labels.settingsShortcuts}</h3>
              <p className="quiet-copy">{labels.shortcutsIntro}</p>
              {labels.shortcutGroups.map(([group, shortcuts]) => <section className="settings-group shortcut-group" key={group} aria-label={group}>
                <h4>{group}</h4>
                <dl className="shortcut-list">
                  {shortcuts.map(([action, keys]) => <div key={action}><dt>{action}</dt><dd><kbd>{keys}</kbd></dd></div>)}
                </dl>
              </section>)}
            </section>}
          </div>
        </aside>}
        <section className="workspace" aria-label={labels.title} inert={sheet !== null || (narrow && sidebarOpen)}>
          <nav className="terminal-tabs" aria-label={labels.terminalTabs}>
            <button className="sidebar-toggle" type="button" aria-label={labels.toggleSidebar} aria-expanded={narrow ? sidebarOpen : !sidebarCollapsed} title={labels.shortcut(labels.toggleSidebar, "Ctrl+Shift+B")} onClick={toggleSidebar}><SidebarIcon /></button>
            <div className="terminal-tab-list" role="tablist" aria-label={labels.terminalTabs}>
              {tabs.map((tab) => {
                const selected = tab.id === activeTab;
                const tabName = tabNames.get(tab.id) ?? labels.localShell;
                const status = tabStatus(tab);
                const ssh = tab.launch.kind === "ssh";
                const production = tab.launch.kind === "ssh" && productionHosts.has(tab.launch.host_id);
                const signal = ssh || status !== "live" ? (status === "starting" ? "connecting" : status) : null;
                const description = [production ? labels.production : "", signal ? labels.tabStatus[status] : ""].filter(Boolean).join(", ");
                return (
                  <div className={"terminal-tab" + (selected ? " active" : "") + (dropTab === tab.id ? " is-tab-drop-target" : "")} key={tab.id} data-status={status} data-prod={production || undefined}
                    data-theme={selected && activePaneTheme === "light" ? "light" : undefined} draggable
                    onDragStart={(event) => { draggedTabRef.current = tab.id; event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-capstan-tab", tab.id); }}
                    onDragOver={(event) => { if (draggedTabRef.current && draggedTabRef.current !== tab.id) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; setDropTab(tab.id); } }}
                    onDragLeave={() => setDropTab(null)}
                    onDragEnd={() => { draggedTabRef.current = null; setDropTab(null); }}
                    onDrop={(event) => {
                      const source = event.dataTransfer.getData("application/x-capstan-tab");
                      if (draggedTabRef.current && source === draggedTabRef.current) { event.preventDefault(); moveTab(draggedTabRef.current, tab.id); }
                      draggedTabRef.current = null;
                      setDropTab(null);
                    }}>
                    <button id={tab.id + "-tab"} type="button" role="tab" aria-selected={selected} aria-controls={tab.id + "-panel"} title={`${tabName} · ${labels.reorderTabHint}`}
                      tabIndex={selected ? 0 : -1} aria-label={description ? `${tabName}, ${description}` : undefined}
                      onClick={withFocus(() => setActiveTab(tab.id))} onKeyDown={(event) => terminalTabKeyDown(event, tab.id)}>
                      <span className="tab-kind" aria-hidden="true">{ssh ? <ServerIcon size={14} /> : <TerminalIcon size={14} />}{signal && <span className="led" data-state={signal} />}</span>
                      <span className="tab-label">{tabName}</span>
                    </button>
                    <button className="terminal-tab-close" type="button" tabIndex={-1} title={labels.shortcut(labels.closeTerminal, "Ctrl+Shift+W")} aria-label={labels.closeTerminal + ": " + tabName} aria-keyshortcuts="Delete Control+Shift+W" onClick={withFocus(() => closeTerminal(tab.id))}><CloseIcon size={14} /></button>
                  </div>
                );
              })}
            </div>
            <div className="tab-actions">
              {activeEntry && <div className="tab-tool-group" role="group" aria-label={labels.workspaceAction}><PaneTools tab={activeEntry} labels={labels} onRequestFocus={() => setFocusRequest((value) => value + 1)} /></div>}
              {vault.unlocked && <div className="tab-tool-group" role="group" aria-label={labels.toolsGroup}>
                <button className="icon-button snippet-open" type="button" aria-label={labels.openSnippets} title={labels.openSnippets} onClick={() => openSheet("snippets")}><SnippetIcon /></button>
                <button className="icon-button tunnel-open" type="button" aria-label={labels.openTunnels} title={labels.openTunnels} onClick={() => openSheet("tunnels")}><TunnelIcon /></button>
              </div>}
              <div className="tab-tool-group" role="group" aria-label={labels.newTerminal}>
              <button className="new-terminal" type="button" title={labels.shortcut(labels.newTerminal, "Ctrl+Shift+T")} aria-label={labels.newTerminal} onClick={withFocus(addTerminal)}><PlusIcon /></button>
              <LocalShellLauncher labels={labels} onLaunch={(options, label) => { addTerminal(options, label); setFocusRequest((value) => value + 1); }} />
              </div>
            </div>
          </nav>
          {!sheet && !sidebarVisible && messages && <div className="workspace-messages">{messages}</div>}
          <section ref={shellRef} className="terminal-shell" aria-label={labels.terminalArea}>
            {/* Keep panel owners in creation order: React StrictMode can replay
                effects when keyed DOM siblings move during tab reordering. */}
            {[...tabs].sort((left, right) => left.id.localeCompare(right.id, undefined, { numeric: true })).map((tab) => {
              const launch = tab.launch;
              const paneSession = tab.paneStates[tab.focusedPaneId]?.session;
              const sshHostId = launch.kind === "ssh" ? launch.host_id : undefined;
              const sshHost = launch.kind === "ssh" ? hosts.find((host) => host.id === launch.host_id) : undefined;
              const sftpHostLabel = sshHost?.name ?? tab.label ?? "";
              const sftpTheme = tab.paneStates[tab.focusedPaneId]?.themeOverride ?? (launch.kind === "ssh" ? launch.theme : "graphite");
              const savedLayout = panelState.tabs[tab.id] ?? panelState.layout;
              const sftpLayout = effectiveLayout(savedLayout, compact, compactExpanded.has(tab.id));
              const tabName = tabNames.get(tab.id) ?? labels.localShell;
              const production = !!sshHostId && productionHosts.has(sshHostId);
              const status = paneStatus(tab, tab.focusedPaneId);
              return (
              <div id={tab.id + "-panel"} className={`terminal-pane${launch.kind === "ssh" && paneSession ? ` terminal-pane-sftp-${sftpLayout.dock}${sftpLayout.collapsed ? " is-collapsed" : ""}` : ""}`} style={launch.kind === "ssh" && paneSession ? { "--sftp-size": `${sftpLayout.size}%` } as CSSProperties : undefined} role="tabpanel" aria-labelledby={tab.id + "-tab"} key={tab.id} hidden={tab.id !== activeTab}>
                <div className="terminal-pane-terminal">
                  <PaneWorkspace tab={tab} visible={windowVisible && tab.id === activeTab} focusRequest={focusRequest} labels={labels} name={tabName} productionHosts={productionHosts}
                    restartTheme={sshHost ? terminalThemeId(sshHost.theme) : undefined} onRequestFocus={() => setFocusRequest((value) => value + 1)} />
                </div>
                {launch.kind === "ssh" && paneSession && <SftpPanel key={paneSession.id} terminal={paneSession.id} locale={locale} hostLabel={sftpHostLabel} theme={sftpTheme} visible={tab.id === activeTab}
                  production={production} ended={status === "ended" || status === "failed"} onLeave={() => setFocusRequest((value) => value + 1)}
                  layout={sftpLayout} preferencesError={panelState.error} onRetryPreferences={() => void panelPreferences.retry()}
                  onLayoutChange={(layout) => {
                    const change = resolveLayoutChange(savedLayout, compact, compactExpanded.has(tab.id), layout);
                    if (change.persist) panelPreferences.set(tab.id, change.persist);
                    setCompactExpanded((current) => {
                      if (current.has(tab.id) === change.expanded) return current;
                      const next = new Set(current);
                      if (change.expanded) next.add(tab.id); else next.delete(tab.id);
                      return next;
                    });
                  }}
                  onSessionChange={(session) => setSftpSessions((current) => {
                    const next = { ...current };
                    if (session === null) delete next[paneSession.id];
                    else if (sshHostId) next[paneSession.id] = { terminal: paneSession.id, hostId: sshHostId, session };
                    return next;
                  })} />}
              </div>
              );
            })}
          </section>
          <RemoteEditors locale={locale} theme={activePaneTheme} hosts={Object.fromEntries(hosts.map((host) => [host.id, host.name]))} sessions={Object.values(sftpSessions)} onRetrySessionRequested={focusTransferHost} />
          <TransferQueue locale={locale} theme={activePaneTheme} hosts={Object.fromEntries(hosts.map((host) => [host.id, host.name]))} production={productionHosts} sessions={Object.values(sftpSessions)} onRetrySessionRequested={focusTransferHost} />
        </section>
      </div>
      {activeLaunch?.kind === "ssh" && <div inert={sheet !== null || (narrow && sidebarOpen)}>
        <ServerStatusBar key={statusTerminal ?? activeLaunch.attempt_id} host={activeHost?.name ?? activeEntry?.label ?? labels.serverStatus.sshHost}
          snapshot={serverStatus} ended={activeEntry !== undefined && ["failed", "ended"].includes(paneStatus(activeEntry, activeEntry.focusedPaneId))}
          labels={labels.serverStatus} />
      </div>}
      <dialog ref={dialogRef} className="trust-dialog" data-alert={challenge?.keyChanged ? "true" : undefined} aria-labelledby="trust-title" onCancel={(event) => { event.preventDefault(); void respondToChallenge(false); }}>
        {challenge && <>
          <h2 id="trust-title"><span className="dialog-mark" aria-hidden="true">{challenge.keyChanged ? <AlertIcon size={18} /> : <ShieldIcon size={18} />}</span>{challenge.keyChanged ? labels.hostKeyChangedTitle : labels.hostKeyTitle}{challengeProduction && <span className="prod-chip">{labels.production}</span>}</h2>
          <p>{challenge.keyChanged
            ? labels.hostKeyChangedSummary(challengeHost?.name ?? challenge.target, challenge.target, challenge.port)
            : labels.hostKeyMessage(challengeHost?.name ?? null, challenge.target, challenge.port)}</p>
          <dl>
            {challenge.keyChanged && <>
              <dt>{labels.knownHostKey}</dt><dd>{challenge.knownAlgorithm}{fingerprint(challenge.knownFingerprint ?? "", null)}</dd>
              <dt>{labels.presentedHostKey}</dt><dd>{challenge.algorithm}{fingerprint(challenge.fingerprint, challenge.knownFingerprint)}</dd>
            </>}
            {!challenge.keyChanged && <>
              <dt>{labels.hostKeyAlgorithm}</dt><dd>{challenge.algorithm}</dd>
              <dt>{labels.hostKeyFingerprint}</dt><dd>{fingerprint(challenge.fingerprint, null)}</dd>
            </>}
          </dl>
          <div className="verify-hint"><KeyIcon size={14} /><div>
            <p>{labels.hostKeyVerifyHint}</p>
            <div className="verify-command"><code>{verifyCommand}</code>
              <button className="icon-button" type="button" title={commandCopied ? labels.commandCopied : labels.copyCommand} aria-label={commandCopied ? labels.commandCopied : labels.copyCommand} onClick={copyVerifyCommand}>{commandCopied ? <CheckIcon size={14} /> : <CopyIcon size={14} />}</button>
            </div>
          </div></div>
          <details><summary>{labels.showPublicKey}</summary><code className="public-key">{challenge.publicKey}</code></details>
          <div className="dialog-actions">
            {challenge.keyChanged
              ? <>
                <button type="button" onClick={() => setChallenge(null)}>{labels.dismissHostKeyChange}</button>
                <button className="ghost-button" type="button" onClick={() => { setChallenge(null); setSettingsTab("keys"); openSheet("settings"); }}>{labels.openHostKeys}</button>
              </>
              : <>
                <button className="ghost-button" type="button" onClick={() => { void respondToChallenge(false); }}>{labels.rejectHostKey}</button>
                <button type="button" className="primary-action" onClick={() => { void respondToChallenge(true); }}>{labels.trustOnceAndSave}</button>
              </>}
          </div>
        </>}
      </dialog>
      <dialog ref={backupDialogRef} className="trust-dialog" aria-labelledby="backup-title" onCancel={(event) => { event.preventDefault(); cancelBackupPreview(); }}>
        {backupPreview && <>
          <h2 id="backup-title"><span className="dialog-mark" aria-hidden="true"><ShieldIcon size={18} /></span>{labels.backupPreviewTitle}</h2>
          <p>{labels.backupPreviewCounts(backupPreview.preview.hostCount, backupPreview.preview.groupCount, backupPreview.preview.credentialCount, backupPreview.preview.trustCount, backupPreview.preview.snippetGroupCount, backupPreview.preview.snippetCount)}</p>
          {backupPreview.preview.duplicateHosts.length > 0 && <>
            <h3>{labels.backupDuplicateHosts}</h3>
            <ul>{backupPreview.preview.duplicateHosts.map((duplicate, index) => <li key={`${duplicate.target}:${duplicate.port}:${index}`}>
              {duplicate.incomingName} → {duplicate.existingName} ({duplicate.target}:{duplicate.port})
            </li>)}</ul>
          </>}
          {backupPreview.preview.trustConflicts.length > 0 && <>
            <h3>{labels.backupTrustConflict}</h3>
            <ul>{backupPreview.preview.trustConflicts.map((conflict) => <li key={`${conflict.target}:${conflict.port}`}>
              {conflict.target}:{conflict.port} · {conflict.existingFingerprint} → {conflict.incomingFingerprint}
            </li>)}</ul>
            <p className="inline-error">{labels.backupTrustConflict}</p>
          </>}
          <div className="dialog-actions">
            <button className="ghost-button" type="button" onClick={cancelBackupPreview}>{labels.cancelEdit}</button>
            <button type="button" className="danger-action" disabled={backupPreview.preview.trustConflicts.length > 0} onClick={() => { void importVaultBackup("overwrite"); }}>{labels.backupOverwrite}</button>
            <button type="button" className="primary-action" disabled={backupPreview.preview.trustConflicts.length > 0} onClick={() => { void importVaultBackup("merge"); }}>{labels.backupMerge}</button>
          </div>
        </>}
      </dialog>
      <CommandPalette open={commandPaletteOpen} labels={labels} actions={paletteActions} onOpenChange={setCommandPaletteOpen} onExecute={executePaletteAction} />
      {confirmElement}
    </main>
  );
}
