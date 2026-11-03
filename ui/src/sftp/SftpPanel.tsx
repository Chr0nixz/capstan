import { useEffect, useId, useMemo, useRef, useState, useSyncExternalStore, type FormEvent, type KeyboardEvent, type PointerEvent, type ReactElement, type ReactNode } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { flushSync } from "react-dom";
import type { TerminalSessionId, TerminalTheme } from "../terminal/contracts";
import { createSftpController, sftpError } from "./controller";
import { createCwdController } from "./cwd";
import { CwdControls, FollowToggle } from "./CwdControls";
import { TreeDeleteDialog } from "./TreeDeleteDialog";
import { transferCopy } from "../transfers/copy";
import { listenTauriEvent } from "../terminal/bridge";
import { armDrop, disarmDrop, enqueueDrop, insideNativeDrop, nativeExports, prepareExport, releaseExport, startExport, type NativeExport } from "../transfers/native";
import { selectAndEnqueue } from "../transfers/selection";
import { releaseTree, resolveTree, selectTreeDownload, selectTreeUpload, type TreeDecision, type TreePlan } from "../transfers/tree";
import { TreeTransferDialog } from "../transfers/TreeTransferDialog";
import { sftpCopy } from "./copy";
import { tauriSftpTransport } from "./transport";
import { remoteEditors } from "../editors/controller";
import type { FileKind, Mutation, RemoteEntryId, Shell, ShellAction, ShellPreview, SftpErrorCode, SftpSessionId } from "./contracts";
import { AlertIcon, ArrowRightIcon, ArrowUpIcon, CheckIcon, CollapseIcon, CopyIcon, DockBottomIcon, DockRightIcon, DownloadIcon, ExportIcon, ExternalEditIcon, EyeIcon, FileIcon, FilePlusIcon, FolderDownloadIcon, FolderIcon, FolderPlusIcon, FolderUploadIcon, FollowIcon, KeyIcon, LinkIcon, MoreIcon, PencilIcon, RefreshIcon, ShellIcon, TrashIcon, UploadIcon } from "../ui/icons";

type Locale = keyof typeof sftpCopy;
type Dock = "right" | "bottom";
type Layout = { dock: Dock; size: number; collapsed: boolean };
type DialogState = { kind: "create"; directory: boolean } | { kind: "rename" } | { kind: "permissions" } | { kind: "confirm"; confirmation: import("./contracts").Confirmation } | { kind: "integration" };
type Props = { terminal: TerminalSessionId; locale: Locale; hostLabel: string; theme: TerminalTheme; layout: Layout; visible?: boolean;
  /** Production hosts get the red identity chip and stricter delete wording. */
  production?: boolean;
  /** The SSH session behind this panel has ended. */
  ended?: boolean;
  /** Escape on a file row hands focus back to the terminal. */
  onLeave?: () => void;
  preferencesError?: "load" | "save" | null; onRetryPreferences?: () => void; onLayoutChange: (layout: Layout) => void; onSessionChange?: (session: SftpSessionId | null) => void };

type MenuItem = { label: string; icon: ReactNode; run: () => void; disabled?: boolean; checked?: boolean; keys?: string; danger?: boolean };

/** Icon-only toolbar key; the visually hidden label keeps its accessible name and text. */
function ToolButton({ label, icon, onClick, disabled, pressed }: { label: string; icon: ReactNode; onClick: () => void; disabled?: boolean; pressed?: boolean }): ReactElement {
  return <button className="icon-button" type="button" title={label} aria-pressed={pressed} onClick={onClick} disabled={disabled}>{icon}<span className="sr-only">{label}</span></button>;
}

function parentPath(path: string): string {
  if (path === "/") return "/";
  const parent = path.replace(/\/+$/, "").lastIndexOf("/");
  return parent <= 0 ? "/" : path.slice(0, parent);
}

function formatMode(mode: number | null): string {
  return mode === null ? "—" : (mode & 0o7777).toString(8).padStart(4, "0");
}

function formatSize(value: string | null): string {
  if (value === null) return "—";
  const number = Number(value);
  if (!Number.isSafeInteger(number)) return value;
  if (number < 1024) return `${number} B`;
  if (number < 1024 * 1024) return `${(number / 1024).toFixed(1)} KiB`;
  if (number < 1024 * 1024 * 1024) return `${(number / (1024 * 1024)).toFixed(1)} MiB`;
  return `${(number / (1024 * 1024 * 1024)).toFixed(1)} GiB`;
}

function formatModified(value: string | null, locale: Locale): string {
  if (value === null) return "—";
  const seconds = Number(value);
  if (!Number.isSafeInteger(seconds) || seconds > 8_640_000_000_000) return value;
  return new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeStyle: "short" }).format(seconds * 1000);
}

/** Column form: month, day and time this year; the full date otherwise. The cell title keeps the long form. */
function formatModifiedShort(value: string | null, locale: Locale): string {
  if (value === null) return "—";
  const seconds = Number(value);
  if (!Number.isSafeInteger(seconds) || seconds > 8_640_000_000_000) return value;
  const date = new Date(seconds * 1000);
  return date.getFullYear() === new Date().getFullYear()
    ? new Intl.DateTimeFormat(locale, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(date)
    : new Intl.DateTimeFormat(locale, { year: "numeric", month: "2-digit", day: "2-digit" }).format(date);
}

/** Splits a file name so a long stem truncates while its extension stays visible. */
export function splitName(name: string): [string, string] {
  const match = /(\.(?:tar\.)?[A-Za-z0-9]{1,8})$/u.exec(name);
  if (!match || match.index < 1) return [name, ""];
  return [name.slice(0, match.index), match[1]];
}

export function SftpPanel({ terminal, locale, hostLabel, theme, layout, visible = true, production = false, ended = false, onLeave, preferencesError, onRetryPreferences, onLayoutChange, onSessionChange }: Props): ReactElement {
  const labels = sftpCopy[locale];
  const controller = useMemo(() => createSftpController(tauriSftpTransport(terminal)), [terminal]);
  const cwd = useMemo(() => createCwdController(terminal, controller), [terminal, controller]);
  const state = useSyncExternalStore(controller.store.subscribe, controller.store.getState, controller.store.getState);
  const [path, setPath] = useState("/");
  const [showHidden, setShowHidden] = useState(false);
  const [aliasesOpen, setAliasesOpen] = useState(false);
  const [copyStatus, setCopyStatus] = useState(false);
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [deleteEntry, setDeleteEntry] = useState<RemoteEntryId | null>(null);
  const [dialogValue, setDialogValue] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const focusReturnRef = useRef<HTMLElement | null>(null);
  const dialogGeneration = useRef(0);
  const transferGeneration = useRef(0);
  const [transferBusy, setTransferBusy] = useState(false);
  const [transferError, setTransferError] = useState<SftpErrorCode | null>(null);
  const [integrationShell, setIntegrationShell] = useState<Shell>("bash");
  const [integrationAction, setIntegrationAction] = useState<ShellAction>("install");
  const [integrationPreview, setIntegrationPreview] = useState<ShellPreview | null>(null);
  const [integrationEntry, setIntegrationEntry] = useState<RemoteEntryId | null>(null);
  const [integrationBusy, setIntegrationBusy] = useState(false);
  const [integrationError, setIntegrationError] = useState<SftpErrorCode | null>(null);
  const [editorBusy, setEditorBusy] = useState(false);
  const [editorError, setEditorError] = useState<SftpErrorCode | null>(null);
  const panelRef = useRef<HTMLElement | null>(null);
  const [exportItem, setExportItem] = useState<NativeExport | null>(null);
  const [treePlans, setTreePlans] = useState<Array<{ plan: TreePlan; session: SftpSessionId }>>([]);
  const treePlansRef = useRef(treePlans), treeAttempts = useRef(new Set<string>());
  treePlansRef.current = treePlans;
  const menuId = useId();
  const menuRef = useRef<HTMLDivElement>(null);
  const moreRef = useRef<HTMLButtonElement>(null);
  const menuAnchor = useRef<{ x: number; y: number; returnTo: HTMLElement | null } | null>(null);
  const [menuScope, setMenuScope] = useState<"panel" | "row">("panel");
  useEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const onToggle = (event: Event): void => {
      const open = (event as Event & { newState?: string }).newState === "open";
      if (open) {
        const anchor = menuAnchor.current;
        const rect = moreRef.current?.getBoundingClientRect();
        const x = anchor ? anchor.x : (rect?.right ?? 0) - menu.offsetWidth;
        const y = anchor ? anchor.y : (rect?.bottom ?? 0) + 4;
        menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - menu.offsetWidth - 4))}px`;
        menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - menu.offsetHeight - 4))}px`;
        menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
      } else {
        const returnTo = menuAnchor.current?.returnTo;
        menuAnchor.current = null;
        if (returnTo?.isConnected && (document.activeElement === document.body || menu.contains(document.activeElement))) returnTo.focus();
      }
    };
    menu.addEventListener("toggle", onToggle);
    return () => menu.removeEventListener("toggle", onToggle);
  });
  const closeMenu = (): void => {
    const menu = menuRef.current;
    try { if (menu?.matches(":popover-open")) menu.hidePopover(); } catch { /* No popover support (tests). */ }
  };
  const openMenuAt = (x: number, y: number, returnTo: HTMLElement | null): void => {
    const menu = menuRef.current;
    if (!menu || typeof menu.showPopover !== "function") return;
    closeMenu();
    menuAnchor.current = { x, y, returnTo };
    // Render the row's items before the popover measures and focuses them.
    flushSync(() => setMenuScope("row"));
    menu.showPopover();
  };
  const menuKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key === "Tab") { closeMenu(); return; }
    const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "ArrowDown" ? items[(index + 1) % items.length] : event.key === "ArrowUp" ? items[(index - 1 + items.length) % items.length]
      : event.key === "Home" ? items[0] : event.key === "End" ? items.at(-1) : undefined;
    if (!next) return;
    event.preventDefault();
    next.focus();
  };

  useEffect(() => {
    transferGeneration.current++;
    cwd.start();
    void controller.start();
    return () => { transferGeneration.current++; onSessionChange?.(null); cwd.dispose(); void controller.dispose(); for (const item of treePlansRef.current) void releaseTree(item.plan).catch(() => undefined); };
  }, [controller, cwd]);
  useEffect(() => {
    const unavailable = state.error === "connection_lost" || state.error === "scope_expired" || state.error === "sftp_unavailable";
    onSessionChange?.(unavailable ? null : state.attached?.sessionId ?? null);
  }, [state.attached?.sessionId, state.error]);
  useEffect(() => {
    cwd.setActivity(visible && !layout.collapsed, transferBusy || dialog !== null || deleteEntry !== null || submitting || treePlans.length > 0);
  }, [cwd, visible, layout.collapsed, transferBusy, dialog, deleteEntry, submitting, treePlans.length]);
  const applyTree = async (decisions: TreeDecision[]): Promise<void> => {
    const current = treePlansRef.current[0];
    if (!current || transferBusy) return;
    setTransferBusy(true); setTransferError(null);
    try {
      const plan = await resolveTree(terminal, current.session, current.plan, decisions);
      setTreePlans((plans) => plans[0]?.plan.token === current.plan.token ? (plan ? [{ plan, session: current.session }, ...plans.slice(1)] : plans.slice(1)) : plans);
    } catch (cause) { setTransferError(sftpError(cause)); }
    finally { setTransferBusy(false); }
  };
  useEffect(() => {
    const first = treePlans[0];
    if (!first || first.plan.conflicts.length || transferBusy) return;
    const key = `${first.plan.token}:${first.plan.revision}`;
    if (treeAttempts.current.has(key)) return;
    treeAttempts.current.add(key);
    void applyTree([]);
  }, [treePlans, transferBusy]);
  useEffect(() => {
    if (!visible || layout.collapsed) return;
    let disposed = false, unlisten: (() => void) | undefined;
    void listenTauriEvent<unknown>("native-transfer-drop", (payload) => {
      const current = controller.store.getState(), rect = panelRef.current?.getBoundingClientRect();
      if (!rect || !insideNativeDrop(payload, rect) || !current.attached || !current.page || current.busy || transferBusy || dialog !== null || deleteEntry !== null || treePlansRef.current.length) return;
      const generation = transferGeneration.current;
      setTransferBusy(true); setTransferError(null);
      const session = current.attached.sessionId;
      void enqueueDrop(terminal, session, current.page.path, payload.token).then((plans) => {
        if (generation === transferGeneration.current) setTreePlans((current) => [...current, ...plans.map((plan) => ({ plan, session }))]);
        else for (const plan of plans) void releaseTree(plan).catch(() => undefined);
      }).catch((cause: unknown) => {
        if (generation === transferGeneration.current) setTransferError(sftpError(cause));
      }).finally(() => { if (generation === transferGeneration.current) setTransferBusy(false); });
    }).then((listener) => { if (disposed) listener(); else unlisten = listener; }).catch(() => undefined);
    return () => { disposed = true; unlisten?.(); };
  }, [controller, terminal, visible, layout.collapsed, transferBusy, dialog, deleteEntry]);
  useEffect(() => {
    if (!visible || layout.collapsed || state.busy || dialog !== null || deleteEntry !== null || treePlans.length || !state.attached || !state.page) return;
    let disposed = false, token: string | undefined;
    void armDrop(terminal, state.attached.sessionId, state.page.path).then((value) => {
      if (disposed) void disarmDrop(value).catch(() => undefined); else token = value;
    }).catch(() => undefined);
    return () => { disposed = true; if (token) void disarmDrop(token).catch(() => undefined); };
  }, [terminal, visible, layout.collapsed, state.busy, dialog, deleteEntry, treePlans.length, state.attached?.sessionId, state.page?.path, state.page?.requestRevision]);
  useEffect(() => {
    if (!exportItem || (exportItem.state !== "preparing" && exportItem.state !== "planning" && exportItem.state !== "ready" && exportItem.state !== "dragging")) return;
    let active = true;
    const timer = window.setInterval(() => { void nativeExports().then((items) => {
      if (active) setExportItem(items.find((item) => item.token === exportItem.token) ?? null);
    }).catch(() => undefined); }, 500);
    return () => { active = false; window.clearInterval(timer); };
  }, [exportItem]);
  const beginTransfer = async (download: boolean): Promise<void> => {
    if (!state.attached || !state.page || transferBusy || state.busy || (download && !state.selected)) return;
    const generation = transferGeneration.current;
    const session = state.attached.sessionId, page = state.page;
    cwd.setActivity(visible && !layout.collapsed, true);
    setTransferBusy(true); setTransferError(null);
    try {
      await selectAndEnqueue(terminal, session, page, locale, download ? state.selected : null, () => {
        const latest = controller.store.getState();
        return generation === transferGeneration.current && latest.attached?.sessionId === session
          && latest.page?.requestRevision === page.requestRevision && latest.page.path === page.path;
      });
    } catch (cause) { if (generation === transferGeneration.current) setTransferError(sftpError(cause)); }
    finally { if (generation === transferGeneration.current) setTransferBusy(false); }
  };
  const beginFolderTransfer = async (): Promise<void> => {
    if (!state.attached || !state.page || transferBusy || state.busy) return;
    const generation = transferGeneration.current;
    const session = state.attached.sessionId, page = state.page;
    cwd.setActivity(visible && !layout.collapsed, true);
    setTransferBusy(true); setTransferError(null);
    try {
      const plan = await selectTreeUpload(terminal, session, page.path, locale);
      if (plan) {
        const latest = controller.store.getState();
        if (generation !== transferGeneration.current || latest.attached?.sessionId !== session || latest.page?.requestRevision !== page.requestRevision || latest.page.path !== page.path) await releaseTree(plan);
        else setTreePlans((plans) => [...plans, { plan, session }]);
      }
    } catch (cause) { if (generation === transferGeneration.current) setTransferError(sftpError(cause)); }
    finally { if (generation === transferGeneration.current) setTransferBusy(false); }
  };
  const beginFolderDownload = async (): Promise<void> => {
    if (!state.attached || !state.page || !state.selected || selected?.metadata.kind !== "directory" || transferBusy || state.busy) return;
    const generation = transferGeneration.current;
    const session = state.attached.sessionId, entry = state.selected;
    cwd.setActivity(visible && !layout.collapsed, true);
    setTransferBusy(true); setTransferError(null);
    try {
      const plan = await selectTreeDownload(terminal, session, entry, locale);
      if (plan) {
        if (generation !== transferGeneration.current || controller.store.getState().attached?.sessionId !== session) await releaseTree(plan);
        else setTreePlans((plans) => [...plans, { plan, session }]);
      }
    } catch (cause) { if (generation === transferGeneration.current) setTransferError(sftpError(cause)); }
    finally { if (generation === transferGeneration.current) setTransferBusy(false); }
  };
  useEffect(() => {
    if (state.page) setPath(state.page.path);
  }, [state.page]);
  const entries = state.page?.entries.filter((entry) => showHidden || !entry.name.startsWith(".")) ?? [];
  const selected = state.page?.entries.find((entry) => entry.id === state.selected);
  const error = state.error ? labels.errors[state.error as SftpErrorCode] : null;
  const preferencesMessage = preferencesError === "load" ? labels.preferencesLoadFailed : labels.preferencesSaveFailed;
  const navigate = async (nextPath: string): Promise<void> => {
    const value = nextPath;
    if (value.length === 0) return;
    cwd.setFollow(false);
    await controller.navigate(value);
  };
  const rememberFocus = (): void => { focusReturnRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; };
  const closeDialog = (): void => { ++dialogGeneration.current; setDialog(null); setSubmitting(false); setIntegrationPreview(null); setIntegrationEntry(null); setIntegrationError(null); setIntegrationBusy(false); };
  const openDialog = (next: DialogState, initial = ""): void => {
    cwd.setActivity(visible && !layout.collapsed, true);
    ++dialogGeneration.current;
    rememberFocus();
    setDialogValue(initial);
    setDialog(next);
  };
  const openIntegration = (): void => {
    if (!selected || selected.metadata.kind !== "file") return;
    setIntegrationShell("bash"); setIntegrationAction("install"); setIntegrationPreview(null); setIntegrationEntry(selected.id); setIntegrationError(null);
    openDialog({ kind: "integration" });
  };
  const openEditor = async (entry = selected): Promise<void> => {
    if (!entry || entry.metadata.kind !== "file" || !state.attached || editorBusy) return;
    setEditorBusy(true); setEditorError(null);
    try {
      const editorId = await controller.editorConfiguration() ?? await controller.selectEditor(locale);
      if (!editorId) return;
      remoteEditors.opened(await controller.beginEditor(entry.id, editorId));
    } catch (cause) { setEditorError(sftpError(cause)); }
    finally { setEditorBusy(false); }
  };
  useEffect(() => {
    if (dialog?.kind !== "integration" || !state.attached || !integrationEntry || integrationPreview || integrationBusy) return;
    const generation = dialogGeneration.current;
    setIntegrationBusy(true); setIntegrationError(null);
    void controller.previewShell(integrationShell, integrationAction, integrationEntry).then((preview) => {
      if (generation === dialogGeneration.current) setIntegrationPreview(preview);
    }).catch((cause) => {
      if (generation === dialogGeneration.current) setIntegrationError(sftpError(cause));
    }).finally(() => {
      if (generation === dialogGeneration.current) setIntegrationBusy(false);
    });
  }, [controller, dialog, integrationAction, integrationBusy, integrationEntry, integrationPreview, integrationShell, state.attached]);
  const changeIntegration = (shell: Shell, action: ShellAction): void => {
    setIntegrationShell(shell); setIntegrationAction(action); setIntegrationPreview(null); setIntegrationError(null);
  };
  const applyIntegration = async (): Promise<void> => {
    if (!integrationPreview || !integrationEntry || integrationBusy) return;
    const generation = dialogGeneration.current;
    setIntegrationBusy(true); setIntegrationError(null);
    try {
      await controller.applyShell(integrationEntry, integrationPreview.token);
      if (generation === dialogGeneration.current) closeDialog();
    } catch (cause) {
      if (generation === dialogGeneration.current) setIntegrationError(sftpError(cause));
    } finally {
      if (generation === dialogGeneration.current) setIntegrationBusy(false);
    }
  };
  const prepareMutation = async (mutation: Mutation): Promise<void> => {
    if (submitting) return;
    cwd.setActivity(visible && !layout.collapsed, true);
    const generation = dialogGeneration.current;
    setSubmitting(true);
    const confirmation = await controller.prepare(mutation);
    if (generation !== dialogGeneration.current) return;
    setSubmitting(false);
    if (confirmation) setDialog({ kind: "confirm", confirmation });
  };
  const openEntry = (entry: typeof selected): void => {
    if (state.busy || !entry) return;
    if (entry.metadata.kind === "file") { void openEditor(entry); return; }
    if (entry.metadata.kind !== "directory" && entry.metadata.kind !== "symlink") return;
    const base = state.page?.path ?? "/";
    void navigate(base === "/" ? `/${entry.name}` : `${base}/${entry.name}`);
  };
  const copyPath = async (value = state.page?.path): Promise<void> => {
    if (!value || !navigator.clipboard) return;
    try {
      await navigator.clipboard.writeText(value);
      setCopyStatus(true);
      window.setTimeout(() => setCopyStatus(false), 1600);
    } catch {
      setCopyStatus(false);
    }
  };
  const submitDialog = async (event: FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault();
    if (!dialog || submitting) return;
    const generation = dialogGeneration.current;
    setSubmitting(true);
    if (dialog.kind === "create") {
      const name = dialogValue;
      if (!name) { setSubmitting(false); return; }
      const created = await controller.create(name, dialog.directory);
      if (generation !== dialogGeneration.current) return;
      if (created) closeDialog();
      else setSubmitting(false);
      return;
    }
    if (dialog.kind === "rename") {
      const name = dialogValue;
      if (!name) { setSubmitting(false); return; }
      const confirmation = await controller.prepare({ kind: "rename", name });
      if (generation !== dialogGeneration.current) return;
      setSubmitting(false);
      if (confirmation) setDialog({ kind: "confirm", confirmation });
      return;
    }
    if (dialog.kind === "permissions") {
      if (!/^[0-7]{3,4}$/.test(dialogValue)) { setSubmitting(false); return; }
      const confirmation = await controller.prepare({ kind: "permissions", mode: Number.parseInt(dialogValue, 8) });
      if (generation !== dialogGeneration.current) return;
      setSubmitting(false);
      if (confirmation) setDialog({ kind: "confirm", confirmation });
    }
  };

  if (layout.collapsed) return <button className="sftp-panel-collapsed" data-theme={theme} type="button" onClick={() => onLayoutChange({ ...layout, collapsed: false })} aria-label={preferencesError ? `${labels.expand}. ${preferencesMessage}` : labels.expand} title={preferencesError ? preferencesMessage : undefined}>
    {preferencesError ? <AlertIcon size={14} /> : <FolderIcon size={14} />}{labels.expand}
  </button>;
  const transferLabels = transferCopy[locale];
  const blocked = transferBusy || state.busy;
  const emptyMessage = !state.busy && !error && state.page && entries.length === 0 && state.page.omittedEntries === 0
    ? state.page.entries.length === 0 && !state.page.hasMore && state.page.pageSequence === 0 ? labels.empty : labels.noVisible
    : null;
  const canEditMode = !!selected && selected.metadata.kind !== "symlink" && selected.metadata.kind !== "other" && selected.metadata.kind !== "unknown";
  const startDelete = (entry = selected): void => {
    if (state.busy || !entry) return;
    rememberFocus();
    if (entry.metadata.kind === "directory") setDeleteEntry(entry.id); else void prepareMutation({ kind: "delete" });
  };
  const startRename = (entry = selected): void => { if (!state.busy && entry) openDialog({ kind: "rename" }, entry.name); };
  const prepareNativeExport = (): void => {
    if (!selected || !state.attached) return;
    setTransferBusy(true); setTransferError(null);
    const session = state.attached.sessionId;
    const generation = transferGeneration.current;
    void prepareExport(terminal, session, selected.id).then(async (result) => {
      if (generation !== transferGeneration.current || controller.store.getState().attached?.sessionId !== session) {
        await releaseTree(result.plan).catch(() => undefined); await releaseExport(result.export.token).catch(() => undefined); return;
      }
      setExportItem(result.export); setTreePlans((plans) => [...plans, { plan: result.plan, session }]);
    }).catch((cause: unknown) => { if (generation === transferGeneration.current) setTransferError(sftpError(cause)); }).finally(() => { if (generation === transferGeneration.current) setTransferBusy(false); });
  };
  const menuItems: Array<MenuItem | "separator"> = [
    { label: transferLabels.uploadFolder, icon: <FolderUploadIcon size={14} />, run: () => void beginFolderTransfer(), disabled: blocked || !state.page },
    { label: transferLabels.downloadFolder, icon: <FolderDownloadIcon size={14} />, run: () => void beginFolderDownload(), disabled: blocked || selected?.metadata.kind !== "directory" },
    { label: transferLabels.prepareExport, icon: <ExportIcon size={14} />, run: prepareNativeExport, disabled: blocked || !selected || !["file", "directory"].includes(selected.metadata.kind) },
    "separator",
    { label: labels.newFile, icon: <FilePlusIcon size={14} />, run: () => openDialog({ kind: "create", directory: false }), disabled: state.busy || !state.page },
    { label: labels.rename, icon: <PencilIcon size={14} />, run: () => startRename(), disabled: state.busy || !selected, keys: "F2" },
    { label: labels.permissions, icon: <KeyIcon size={14} />, run: () => openDialog({ kind: "permissions" }, formatMode(selected?.metadata.permissions ?? null)), disabled: state.busy || !canEditMode },
    { label: labels.remove, icon: <TrashIcon size={14} />, run: () => startDelete(), disabled: state.busy || !selected, keys: "Del", danger: true },
    "separator",
    { label: labels.edit, icon: <ExternalEditIcon size={14} />, run: () => void openEditor(), disabled: state.busy || editorBusy || selected?.metadata.kind !== "file" },
    { label: labels.shellIntegration, icon: <ShellIcon size={14} />, run: openIntegration, disabled: state.busy || selected?.metadata.kind !== "file" },
    "separator",
    { label: copyStatus ? labels.copied : labels.copy, icon: copyStatus ? <CheckIcon size={14} className="check-pop" /> : <CopyIcon size={14} />, run: () => { void copyPath(); }, disabled: !state.page },
    { label: labels.hidden, icon: <EyeIcon size={14} />, run: () => setShowHidden((value) => !value), checked: showHidden },
    { label: labels.cwdAliasesMenu, icon: <FollowIcon size={14} />, run: () => {
      setAliasesOpen(true);
      requestAnimationFrame(() => panelRef.current?.querySelector<HTMLInputElement>(".sftp-cwd-aliases input")?.focus());
    } },
  ];
  const entryPath = (name: string): string => { const base = state.page?.path ?? "/"; return base === "/" ? `/${name}` : `${base}/${name}`; };
  const rowItems = (entry: NonNullable<typeof selected>): Array<MenuItem | "separator"> => {
    const kind = entry.metadata.kind;
    const opens = kind === "directory" || kind === "symlink";
    return [
      opens ? { label: labels.go, icon: <FolderIcon size={14} />, run: () => openEntry(entry), disabled: state.busy, keys: "Enter" }
        : { label: labels.edit, icon: <ExternalEditIcon size={14} />, run: () => void openEditor(entry), disabled: state.busy || editorBusy || kind !== "file", keys: "Enter" },
      kind === "directory"
        ? { label: transferLabels.downloadFolder, icon: <FolderDownloadIcon size={14} />, run: () => void beginFolderDownload(), disabled: blocked }
        : { label: transferLabels.download, icon: <DownloadIcon size={14} />, run: () => void beginTransfer(true), disabled: blocked || kind !== "file" },
      { label: transferLabels.prepareExport, icon: <ExportIcon size={14} />, run: prepareNativeExport, disabled: blocked || !["file", "directory"].includes(kind) },
      "separator",
      { label: labels.rename, icon: <PencilIcon size={14} />, run: () => startRename(entry), disabled: state.busy, keys: "F2" },
      { label: labels.permissions, icon: <KeyIcon size={14} />, run: () => openDialog({ kind: "permissions" }, formatMode(entry.metadata.permissions)), disabled: state.busy || !canEditMode },
      { label: labels.remove, icon: <TrashIcon size={14} />, run: () => startDelete(entry), disabled: state.busy, keys: "Del", danger: true },
      "separator",
      { label: copyStatus ? labels.copied : labels.copy, icon: copyStatus ? <CheckIcon size={14} className="check-pop" /> : <CopyIcon size={14} />, run: () => { void copyPath(entryPath(entry.name)); } },
    ];
  };
  const visibleMenu = menuScope === "row" && selected ? rowItems(selected) : menuItems;
  const rowKeyDown = (event: KeyboardEvent<HTMLButtonElement>, entry: (typeof entries)[number]): void => {
    if (event.key === "Enter") { event.preventDefault(); openEntry(entry); return; }
    if (event.key === "Escape" && onLeave) { event.preventDefault(); onLeave(); return; }
    if (event.key === "Delete") { event.preventDefault(); controller.select(entry.id); startDelete(entry); return; }
    if (event.key === "F2") { event.preventDefault(); controller.select(entry.id); startRename(entry); return; }
    if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
      event.preventDefault();
      controller.select(entry.id);
      const rect = event.currentTarget.getBoundingClientRect();
      openMenuAt(rect.left + 12, rect.bottom, event.currentTarget);
      return;
    }
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      const buttons = [...(event.currentTarget.closest("tbody")?.querySelectorAll<HTMLButtonElement>(".sftp-entry-name") ?? [])];
      const index = buttons.indexOf(event.currentTarget);
      const nextIndex = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : Math.min(buttons.length - 1, Math.max(0, index + (event.key === "ArrowDown" ? 1 : -1)));
      buttons[nextIndex]?.focus();
      if (entries[nextIndex]) controller.select(entries[nextIndex].id);
    }
  };
  const resizeKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    const step = event.key === "ArrowLeft" || event.key === "ArrowUp" ? 2 : event.key === "ArrowRight" || event.key === "ArrowDown" ? -2 : 0;
    const size = event.key === "Home" ? 20 : event.key === "End" ? 60 : Math.min(60, Math.max(20, layout.size + step));
    if (!step && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    if (size !== layout.size) onLayoutChange({ ...layout, size });
  };
  const startResize = (event: PointerEvent<HTMLDivElement>): void => {
    const panel = panelRef.current, pane = panel?.parentElement;
    if (!panel || !pane || event.button !== 0) return;
    event.preventDefault();
    const handle = event.currentTarget, paneRect = pane.getBoundingClientRect();
    // The right dock stacks under the terminal in compact windows.
    const stacked = panel.getBoundingClientRect().top > paneRect.top + 1;
    let size = layout.size;
    handle.setPointerCapture(event.pointerId);
    handle.dataset.dragging = "true";
    const move = (moveEvent: globalThis.PointerEvent): void => {
      const value = stacked ? (paneRect.bottom - moveEvent.clientY) / paneRect.height : (paneRect.right - moveEvent.clientX) / paneRect.width;
      size = Math.round(Math.min(60, Math.max(20, value * 100)));
      pane.style.setProperty("--sftp-size", `${size}%`);
    };
    const end = (): void => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", end);
      handle.removeEventListener("pointercancel", end);
      delete handle.dataset.dragging;
      // Persist once, at the end of the drag.
      if (size !== layout.size) onLayoutChange({ ...layout, size });
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end);
    handle.addEventListener("pointercancel", end);
  };
  const otherDock: Dock = layout.dock === "right" ? "bottom" : "right";

  return <>
  {deleteEntry && state.attached && <TreeDeleteDialog key={`${state.attached.sessionId}:${deleteEntry}`} terminal={terminal} session={state.attached.sessionId} entry={deleteEntry} hostLabel={hostLabel} production={production} locale={locale} theme={theme} onClose={() => setDeleteEntry(null)} onChanged={() => { void controller.navigate(state.page?.path ?? path); }} />}
  {treePlans[0] && <TreeTransferDialog plan={treePlans[0].plan} hostLabel={hostLabel} locale={locale} theme={theme} pending={transferBusy} error={transferError ? labels.errors[transferError] : null} onDecision={(decisions) => void applyTree(decisions)} onClose={() => {
    const first = treePlansRef.current[0]; if (!first || transferBusy) return;
    setTreePlans((plans) => plans.slice(1)); void releaseTree(first.plan).catch(() => undefined);
  }} />}
  <section ref={panelRef} className={`sftp-panel sftp-dock-${layout.dock}`} data-theme={theme} aria-label={labels.title}>
    <div className="sftp-splitter" role="separator" tabIndex={0} aria-label={labels.panelSize} title={labels.panelSize}
      aria-orientation={layout.dock === "right" ? "vertical" : "horizontal"} aria-valuemin={20} aria-valuemax={60} aria-valuenow={layout.size}
      onPointerDown={startResize} onKeyDown={resizeKeyDown} />
    <header className="sftp-panel-header">
      <span className="sftp-title"><FolderIcon size={14} /><strong>{labels.title}</strong>{production && <span className="prod-chip">{labels.production}</span>}<span className="sftp-host" title={hostLabel}>{hostLabel}</span></span>
      <div className="sftp-toolbar sftp-mutation-toolbar">
        <ToolButton label={transferLabels.upload} icon={<UploadIcon />} onClick={() => void beginTransfer(false)} disabled={blocked || !state.page} />
        <ToolButton label={transferLabels.download} icon={<DownloadIcon />} onClick={() => void beginTransfer(true)} disabled={blocked || selected?.metadata.kind !== "file"} />
        <ToolButton label={labels.newDirectory} icon={<FolderPlusIcon />} onClick={() => openDialog({ kind: "create", directory: true })} disabled={state.busy || !state.page} />
        <button ref={moreRef} className="icon-button" type="button" title={labels.moreActions} aria-haspopup="menu" popoverTarget={menuId} onClick={() => { menuAnchor.current = null; flushSync(() => setMenuScope("panel")); }}><MoreIcon /><span className="sr-only">{labels.moreActions}</span></button>
        <div ref={menuRef} id={menuId} className="sftp-menu" popover="auto" role="menu" aria-label={labels.moreActions} data-theme={theme} onKeyDown={menuKeyDown}>
          {visibleMenu.map((item, index) => item === "separator" ? <div className="sftp-menu-separator" role="separator" key={`separator-${index}`} /> : <button key={item.label} type="button"
            role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"} aria-checked={item.checked} className={item.danger ? "is-danger" : undefined}
            disabled={item.disabled} onClick={() => { closeMenu(); item.run(); }}>
            {item.icon}<span className="sftp-menu-label" data-keys={item.keys}>{item.label}</span>{item.checked && <CheckIcon size={14} className="sftp-menu-check" />}
          </button>)}
        </div>
      </div>
      <div className="sftp-panel-header-actions">
        <ToolButton label={otherDock === "bottom" ? labels.dockBottom : labels.dockRight} icon={otherDock === "bottom" ? <DockBottomIcon /> : <DockRightIcon />} onClick={() => onLayoutChange({ ...layout, dock: otherDock })} />
        <ToolButton label={labels.collapse} icon={<CollapseIcon />} onClick={() => onLayoutChange({ ...layout, collapsed: true })} />
      </div>
    </header>
    <div className="sftp-controls">
    {ended && <p className="sftp-status sftp-ended" role="status">{labels.sessionEnded}</p>}
    {preferencesError && <div className="sftp-error" role="alert"><span>{preferencesMessage}</span><button type="button" onClick={onRetryPreferences}>{labels.preferencesRetry}</button></div>}
    <form className="sftp-path-form" onSubmit={(event) => { event.preventDefault(); void navigate(path); }}>
      <ToolButton label={labels.parent} icon={<ArrowUpIcon />} onClick={() => void navigate(parentPath(state.page?.path ?? path))} disabled={state.busy || !state.page} />
      <ToolButton label={labels.refresh} icon={<RefreshIcon />} onClick={() => void controller.navigate(state.page?.path ?? path)} disabled={state.busy} />
      <div className="sftp-path-field">
        <label className="sr-only" htmlFor={`sftp-path-${terminal}`}>{labels.path}</label>
        <input id={`sftp-path-${terminal}`} value={path} onChange={(event) => setPath(event.target.value)} autoComplete="off" spellCheck={false} />
        <button className="icon-button sftp-path-go" type="submit" title={labels.go} disabled={state.busy}><ArrowRightIcon size={14} /><span className="sr-only">{labels.go}</span></button>
      </div>
      <FollowToggle controller={cwd} locale={locale} ended={ended} />
    </form>
    <CwdControls controller={cwd} locale={locale} ended={ended} aliasesOpen={aliasesOpen} onAliasesOpenChange={setAliasesOpen} />
    {editorError && <p className="sftp-error" role="alert">{labels.errors[editorError]}</p>}
    {transferError && <p className="sftp-error" role="alert">{labels.errors[transferError]}</p>}
    {error && <div className="sftp-error" role="alert"><span>{error}</span><button type="button" disabled={state.busy} onClick={() => void controller.start()}>{labels.retry}</button></div>}
    {!!state.page?.omittedEntries && <p className="sftp-status" role="status">{labels.omitted(state.page.omittedEntries)}</p>}
    {exportItem && <div className="sftp-export-row">
      <button type="button" className={exportItem.state === "ready" ? "export-ready" : undefined} draggable={exportItem.state === "ready"} disabled={exportItem.state !== "ready"} onDragStart={(event) => {
        event.preventDefault();
        if (!state.attached || exportItem.state !== "ready") return;
        void startExport(terminal, state.attached.sessionId, exportItem.token).catch((cause: unknown) => setTransferError(sftpError(cause)));
      }}><ExportIcon size={14} /><span>{exportItem.state === "ready" ? transferLabels.dragExport : transferLabels.exportStates[exportItem.state]} · {exportItem.name}</span></button>
    </div>}
    </div>
    {state.busy && <><div className="sftp-busy" aria-hidden="true" /><p className="sr-only" role="status">{labels.loading}</p></>}
    <div className="sftp-table-wrap">
      <table className="sftp-table" aria-label={labels.title}>
        <thead><tr><th>{labels.name}</th><th>{labels.size}</th><th>{labels.type}</th><th>{labels.modified}</th></tr></thead>
        <tbody>{entries.map((entry) => {
          const [head, tail] = splitName(entry.name);
          return <tr key={entry.id} className={entry.id === state.selected ? "selected" : ""} aria-selected={entry.id === state.selected}
          onClick={(event) => { if (!state.busy && !(event.target as HTMLElement).closest("button")) controller.select(entry.id); }}
          onDoubleClick={(event) => { if (!(event.target as HTMLElement).closest("button")) openEntry(entry); }}
          onContextMenu={(event) => { if (state.busy) return; event.preventDefault(); controller.select(entry.id); openMenuAt(event.clientX, event.clientY, event.currentTarget.querySelector<HTMLElement>(".sftp-entry-name")); }}>
          <td><span className="sftp-entry" data-kind={entry.metadata.kind}>{entry.metadata.kind === "directory" ? <FolderIcon size={14} /> : entry.metadata.kind === "symlink" ? <LinkIcon size={14} /> : <FileIcon size={14} />}
            <button type="button" className="sftp-entry-name" title={entry.name} disabled={state.busy} onClick={() => controller.select(entry.id)} onDoubleClick={() => openEntry(entry)} onKeyDown={(event) => rowKeyDown(event, entry)}>
              <span className="sftp-name-head">{head}</span>{tail && <span className="sftp-name-tail">{tail}</span>}
            </button></span></td>
          <td title={entry.metadata.size ?? undefined}>{formatSize(entry.metadata.size)}</td>
          <td title={`${labels.kinds[entry.metadata.kind as FileKind]} · ${formatMode(entry.metadata.permissions)}`}>{labels.kinds[entry.metadata.kind as FileKind]} · <span className="mono">{formatMode(entry.metadata.permissions)}</span></td>
          <td title={formatModified(entry.metadata.modifiedSeconds, locale)}>{formatModifiedShort(entry.metadata.modifiedSeconds, locale)}</td>
        </tr>; })}</tbody>
      </table>
      {emptyMessage && <div className="sftp-empty"><FolderIcon size={22} /><p>{emptyMessage}</p></div>}
    </div>
    <footer className="sftp-footer">
      <p className="drop-hint"><UploadIcon size={12} /><span>{transferLabels.dropHint}</span></p>
      {state.page && (state.page.hasMore || state.page.pageSequence > 0) && <nav className="sftp-pagination" aria-label={labels.title}>
        <button type="button" onClick={() => void controller.previous()} disabled={state.busy || state.page.pageSequence === 0}>{labels.previous}</button>
        <span>{labels.page(state.page.pageSequence + 1)}</span>
        <button type="button" onClick={() => void controller.next()} disabled={state.busy || !state.page.hasMore}>{labels.next}</button>
      </nav>}
    </footer>
  </section>
  <Dialog.Root open={dialog !== null} onOpenChange={(open) => { if (!open) closeDialog(); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="sftp-dialog-overlay" />
      <Dialog.Content className={"sftp-dialog" + (dialog?.kind === "confirm" ? " confirm-dialog" : "")} role={dialog?.kind === "confirm" ? "alertdialog" : "dialog"}
        data-tone={dialog?.kind === "confirm" ? (dialog.confirmation.mutation.kind === "delete" ? "danger" : "primary") : undefined} data-theme={theme}
        onCloseAutoFocus={(event) => { event.preventDefault(); focusReturnRef.current?.focus(); }}>
        {dialog && <>
          {dialog.kind === "confirm" ? <Dialog.Title><span className="dialog-mark" aria-hidden="true">{dialog.confirmation.mutation.kind === "delete" ? <TrashIcon size={16} /> : <AlertIcon size={16} />}</span>{
            (dialog.confirmation.mutation.kind === "delete" ? labels.deleteTitle : labels.changeTitle)(dialog.confirmation.path.split("/").filter(Boolean).at(-1) ?? dialog.confirmation.path, hostLabel)}</Dialog.Title>
            : <Dialog.Title>{dialog.kind === "create" ? labels.createTitle(dialog.directory) : dialog.kind === "rename" ? labels.rename : dialog.kind === "permissions" ? labels.permissions : labels.shellIntegrationTitle}</Dialog.Title>}
          {dialog.kind === "confirm" ? <>
            <Dialog.Description>{dialog.confirmation.mutation.kind === "delete"
              ? [labels.deleteBody, dialog.confirmation.metadata.kind === "directory" ? labels.deleteScopeNote : ""].filter(Boolean).join(" ")
              : dialog.confirmation.mutation.kind === "rename" ? labels.renameTarget(dialog.confirmation.mutation.name)
                : labels.permissionChange(formatMode(dialog.confirmation.metadata.permissions), formatMode(dialog.confirmation.mutation.mode))}</Dialog.Description>
            <p className="confirm-target">{production && <span className="prod-chip">{labels.production}</span>}<span className="confirm-host">{hostLabel}</span><code>{dialog.confirmation.path}</code></p>
            {error && <p className="sftp-dialog-error" role="alert">{error}</p>}
            <div className="sftp-dialog-actions">
              <Dialog.Close asChild><button className="ghost-button" type="button">{labels.cancel}</button></Dialog.Close>
              <button type="button" className={dialog.confirmation.mutation.kind === "delete" ? "danger-action" : "primary-action"} disabled={submitting} onClick={() => {
                if (submitting) return;
                setSubmitting(true);
                // Failed operations also consume tokens; keep the error in the panel.
                void controller.apply(dialog.confirmation).then(() => closeDialog());
              }}>{dialog.confirmation.mutation.kind === "delete" ? labels.deleteKind(labels.kinds[dialog.confirmation.metadata.kind]) : labels.apply}</button>
            </div>
          </> : dialog.kind === "integration" ? <div className="sftp-integration-dialog">
            <Dialog.Description>{labels.shellIntegrationHint}</Dialog.Description>
            <label>{labels.shell}
              <select value={integrationShell} onChange={(event) => changeIntegration(event.target.value as Shell, integrationAction)} disabled={integrationBusy}>
                <option value="bash">Bash</option><option value="zsh">Zsh</option>
              </select>
            </label>
            <label>{labels.action}
              <select value={integrationAction} onChange={(event) => changeIntegration(integrationShell, event.target.value as ShellAction)} disabled={integrationBusy}>
                <option value="install">{labels.install}</option><option value="uninstall">{labels.uninstall}</option>
              </select>
            </label>
            {integrationBusy && <p className="sftp-status" role="status">{labels.previewLoading}</p>}
            {integrationError && <p className="sftp-dialog-error" role="alert">{labels.errors[integrationError]}</p>}
            {integrationPreview && <>
              <p>{integrationPreview.addedLines === 0 && integrationPreview.removedLines === 0 ? labels.noChanges : labels.previewSummary(integrationPreview.addedLines, integrationPreview.removedLines)}</p>
              <code>{integrationPreview.host}</code>
            </>}
            <div className="sftp-dialog-actions">
              <Dialog.Close asChild><button type="button" disabled={integrationBusy}>{labels.cancel}</button></Dialog.Close>
              <button type="button" className="sftp-confirm-action" disabled={integrationBusy || !integrationPreview || !integrationPreview.addedLines && !integrationPreview.removedLines} onClick={() => void applyIntegration()}>{labels.applyIntegration}</button>
            </div>
          </div> : <form onSubmit={(event) => { void submitDialog(event); }}>
            <Dialog.Description>{dialog.kind === "permissions" ? labels.chooseMode : labels.chooseName}</Dialog.Description>
            <input aria-label={dialog.kind === "permissions" ? labels.chooseMode : labels.chooseName} autoFocus value={dialogValue} onChange={(event) => setDialogValue(event.target.value)} maxLength={dialog.kind === "permissions" ? 4 : 255} required />
            {error && <p className="sftp-dialog-error" role="alert">{error}</p>}
            {dialog.kind === "permissions" && !/^[0-7]{3,4}$/.test(dialogValue) && <p className="sftp-dialog-hint">{labels.chooseMode}</p>}
            <div className="sftp-dialog-actions">
              <Dialog.Close asChild><button type="button">{labels.cancel}</button></Dialog.Close>
              <button type="submit" className="sftp-confirm-action" disabled={submitting || !dialogValue}>{labels.apply}</button>
            </div>
          </form>}
        </>}
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>
  </>;
}
