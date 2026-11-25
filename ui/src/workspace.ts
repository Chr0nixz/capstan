import { create } from "zustand";
import type { HostId, LocalTerminalOptions, TabId, TerminalLaunch, TerminalSnapshot, TerminalTheme } from "./terminal/contracts";
import type { PaneId, PaneNode } from "./panes/contracts";
import { findPane, paneLeaves, removePane, replacePane, resizeSplit, type PaneLeaf, type SplitDirection } from "./panes/tree";

export type PaneState = {
  session?: Pick<TerminalSnapshot, "id" | "state" | "exit_code" | "error_code">;
  failed?: boolean;
  themeOverride?: TerminalTheme;
};
export type TerminalTab = {
  id: TabId;
  label?: string;
  /** Projection of the focused leaf, retained for callers opening local shells. */
  launch: TerminalLaunch;
  root: PaneNode;
  focusedPaneId: PaneId;
  paneStates: Record<string, PaneState>;
  broadcastEnabled: boolean;
  broadcastPaneIds: PaneId[];
  /** The target set the user confirmed; any selection change clears it. */
  broadcastArmedPaneIds: PaneId[];
};
export type TabStatus = "starting" | "connecting" | "live" | "ended" | "failed";

export function paneStatus(tab: TerminalTab, paneId: PaneId): TabStatus {
  const pane = findPane(tab.root, paneId), state = tab.paneStates[paneId];
  if (!pane || state?.failed || state?.session?.state === "error") return "failed";
  if (!state?.session) return pane.launch.kind === "ssh" ? "connecting" : "starting";
  return state.session.state === "running" ? "live" : "ended";
}

/** A live sibling keeps the tab live; each pane still shows its own final state. */
export function tabStatus(tab: TerminalTab): TabStatus {
  const statuses = paneLeaves(tab.root).map((pane) => paneStatus(tab, pane.paneId));
  return (["live", "connecting", "starting", "failed", "ended"] as const).find((status) => statuses.includes(status)) ?? "ended";
}

/**
 * What closing a tab would end that the user may not mean to: a running session
 * on a production host, or more than one running session at once.
 */
export type CloseRisk = { productionPaneIds: PaneId[]; runningPaneIds: PaneId[] };
export function closeRisk(tab: TerminalTab, productionHosts: ReadonlySet<HostId>): CloseRisk | null {
  // A connection still being made has nothing running yet; closing it only cancels.
  const running = paneLeaves(tab.root).filter((pane) => paneStatus(tab, pane.paneId) === "live");
  const production = running.filter((pane) => pane.launch.kind === "ssh" && productionHosts.has(pane.launch.host_id));
  return production.length > 0 || running.length > 1
    ? { productionPaneIds: production.map((pane) => pane.paneId), runningPaneIds: running.map((pane) => pane.paneId) }
    : null;
}

/** A closed tab's layout. Reopening it starts new sessions in the same panes. */
type ClosedTab = { index: number; label?: string; root: PaneNode; themes: (TerminalTheme | undefined)[]; focusedIndex: number };
const CLOSED_TAB_LIMIT = 10;

type Workspace = {
  tabs: TerminalTab[];
  activeTab: TabId;
  nextId: number;
  closedTabs: ClosedTab[];
  addTerminal: (options?: LocalTerminalOptions, label?: string) => void;
  addSshTerminal: (hostId: HostId, label: string, theme: TerminalTheme) => void;
  openSshHost: (hostId: HostId, label: string, theme: TerminalTheme, newTab?: boolean) => void;
  restartTab: (id: TabId, theme?: TerminalTheme) => void;
  restartPane: (tab: TabId, pane: PaneId, theme?: TerminalTheme) => void;
  splitPane: (tab: TabId, pane: PaneId, direction: SplitDirection) => void;
  closePane: (tab: TabId, pane: PaneId) => void;
  focusPane: (tab: TabId, pane: PaneId) => void;
  resizePaneSplit: (tab: TabId, split: string, ratio: number) => void;
  setBroadcast: (tab: TabId, enabled: boolean) => void;
  toggleBroadcastPane: (tab: TabId, pane: PaneId) => void;
  armBroadcast: (tab: TabId, panes: readonly PaneId[]) => void;
  setPaneTheme: (tab: TabId, pane: PaneId, theme: TerminalTheme) => void;
  moveTab: (id: TabId, target: TabId) => void;
  closeTerminal: (id: TabId) => void;
  reopenTerminal: () => void;
  selectTab: (id: TabId) => void;
  updateSession: (tab: TabId, snapshot: TerminalSnapshot, pane?: PaneId, launch?: TerminalLaunch) => void;
  failTab: (tab: TabId, pane?: PaneId, launch?: TerminalLaunch) => void;
};

const freshLaunch = (launch: TerminalLaunch, theme?: TerminalTheme): TerminalLaunch => launch.kind === "ssh"
  ? { ...launch, attempt_id: globalThis.crypto.randomUUID(), theme: theme ?? launch.theme }
  : { kind: "local", options: { ...launch.options } };

function createTab(index: number, launch: TerminalLaunch, label?: string): TerminalTab {
  const paneId = globalThis.crypto.randomUUID() as PaneId;
  return {
    id: ("tab-" + index) as TabId, label, launch,
    root: { kind: "pane", paneId, launch }, focusedPaneId: paneId, paneStates: {},
    broadcastEnabled: false, broadcastPaneIds: [], broadcastArmedPaneIds: [],
  };
}

function focus(tab: TerminalTab, id: PaneId): TerminalTab {
  const pane = findPane(tab.root, id);
  return pane ? { ...tab, focusedPaneId: id, launch: pane.launch } : tab;
}

function restarted(tab: TerminalTab, id: PaneId, theme?: TerminalTheme): TerminalTab {
  const pane = findPane(tab.root, id);
  if (!pane) return tab;
  const launch = freshLaunch(pane.launch, theme), paneStates = { ...tab.paneStates };
  const themeOverride = paneStates[id]?.themeOverride;
  delete paneStates[id];
  if (themeOverride) paneStates[id] = { themeOverride };
  return focus({ ...tab, root: replacePane(tab.root, id, { ...pane, launch }), paneStates }, tab.focusedPaneId);
}

function remembered(tab: TerminalTab, index: number): ClosedTab {
  const leaves = paneLeaves(tab.root);
  return {
    index, label: tab.label, root: tab.root,
    themes: leaves.map((pane) => tab.paneStates[pane.paneId]?.themeOverride),
    focusedIndex: Math.max(0, leaves.findIndex((pane) => pane.paneId === tab.focusedPaneId)),
  };
}

function renewed(root: PaneNode): PaneNode {
  return root.kind === "pane"
    ? { kind: "pane", paneId: globalThis.crypto.randomUUID() as PaneId, launch: freshLaunch(root.launch) }
    : { ...root, first: renewed(root.first), second: renewed(root.second) };
}

function closedTab(state: Workspace, id: TabId): Partial<Workspace> {
  const index = state.tabs.findIndex((tab) => tab.id === id);
  if (index < 0) return state;
  const tabs = state.tabs.filter((tab) => tab.id !== id);
  const closedTabs = [...state.closedTabs, remembered(state.tabs[index], index)].slice(-CLOSED_TAB_LIMIT);
  if (tabs.length === 0) {
    const added = createTab(state.nextId, { kind: "local", options: {} });
    return { tabs: [added], activeTab: added.id, nextId: state.nextId + 1, closedTabs };
  }
  return { tabs, activeTab: state.activeTab === id ? tabs[Math.min(index, tabs.length - 1)].id : state.activeTab, closedTabs };
}

const initial = createTab(1, { kind: "local", options: {} });
export const useWorkspace = create<Workspace>((set) => {
  const changeTab = (id: TabId, change: (tab: TerminalTab) => TerminalTab): void => set((state) => {
    const current = state.tabs.find((tab) => tab.id === id);
    if (!current) return state;
    const next = change(current);
    return next === current ? state : { tabs: state.tabs.map((tab) => tab.id === id ? next : tab) };
  });
  return {
    tabs: [initial], activeTab: initial.id, nextId: 2, closedTabs: [],
    addTerminal: (options = {}, label) => set((state) => {
      const added = createTab(state.nextId, { kind: "local", options: { ...options } }, label);
      return { tabs: [...state.tabs, added], activeTab: added.id, nextId: state.nextId + 1 };
    }),
    addSshTerminal: (hostId, label, theme) => set((state) => {
      const added = createTab(state.nextId, { kind: "ssh", host_id: hostId, attempt_id: globalThis.crypto.randomUUID(), theme }, label);
      return { tabs: [...state.tabs, added], activeTab: added.id, nextId: state.nextId + 1 };
    }),
    openSshHost: (hostId, label, theme, newTab = false) => set((state) => {
      const existing = newTab ? [] : state.tabs.filter((tab) => paneLeaves(tab.root).some((pane) => pane.launch.kind === "ssh" && pane.launch.host_id === hostId));
      const target = existing.find((tab) => tab.id === state.activeTab)
        ?? existing.find((tab) => tabStatus(tab) === "live" || tabStatus(tab) === "connecting") ?? existing[0];
      if (target) {
        const pane = paneLeaves(target.root).find((pane) => pane.launch.kind === "ssh" && pane.launch.host_id === hostId
          && (paneStatus(target, pane.paneId) === "live" || paneStatus(target, pane.paneId) === "connecting"))
          ?? findPane(target.root, target.focusedPaneId)!;
        const status = paneStatus(target, pane.paneId);
        const selected = focus(status === "ended" || status === "failed" ? restarted(target, pane.paneId, theme) : target, pane.paneId);
        return { tabs: state.tabs.map((tab) => tab.id === target.id ? selected : tab), activeTab: target.id };
      }
      const added = createTab(state.nextId, { kind: "ssh", host_id: hostId, attempt_id: globalThis.crypto.randomUUID(), theme }, label);
      return { tabs: [...state.tabs, added], activeTab: added.id, nextId: state.nextId + 1 };
    }),
    restartTab: (id, theme) => changeTab(id, (tab) => restarted(tab, tab.focusedPaneId, theme)),
    restartPane: (id, pane, theme) => changeTab(id, (tab) => restarted(tab, pane, theme)),
    splitPane: (id, paneId, direction) => changeTab(id, (tab) => {
      const pane = findPane(tab.root, paneId);
      if (!pane) return tab;
      const added: PaneLeaf = { kind: "pane", paneId: globalThis.crypto.randomUUID() as PaneId, launch: freshLaunch(pane.launch) };
      const paneStates = tab.paneStates[paneId]?.themeOverride
        ? { ...tab.paneStates, [added.paneId]: { themeOverride: tab.paneStates[paneId].themeOverride } }
        : tab.paneStates;
      return focus({ ...tab, root: replacePane(tab.root, paneId, { kind: "split", direction, ratio: 0.5, first: pane, second: added }), paneStates }, added.paneId);
    }),
    closePane: (id, paneId) => set((state) => {
      const tab = state.tabs.find((tab) => tab.id === id);
      if (!tab || !findPane(tab.root, paneId)) return state;
      const root = removePane(tab.root, paneId);
      if (!root) return closedTab(state, id);
      const paneStates = { ...tab.paneStates };
      delete paneStates[paneId];
      const next = focus({ ...tab, root, paneStates, broadcastPaneIds: tab.broadcastPaneIds.filter((id) => id !== paneId), broadcastArmedPaneIds: [] },
        tab.focusedPaneId === paneId ? paneLeaves(root)[0].paneId : tab.focusedPaneId);
      return { tabs: state.tabs.map((item) => item.id === id ? next : item) };
    }),
    focusPane: (id, pane) => changeTab(id, (tab) => tab.focusedPaneId === pane ? tab : focus(tab, pane)),
    resizePaneSplit: (id, split, ratio) => changeTab(id, (tab) => {
      const root = resizeSplit(tab.root, split, ratio);
      return root === tab.root ? tab : { ...tab, root };
    }),
    setBroadcast: (id, enabled) => changeTab(id, (tab) => ({ ...tab, broadcastEnabled: enabled, broadcastArmedPaneIds: [] })),
    toggleBroadcastPane: (id, pane) => changeTab(id, (tab) => !findPane(tab.root, pane) ? tab : {
      ...tab, broadcastPaneIds: tab.broadcastPaneIds.includes(pane) ? tab.broadcastPaneIds.filter((id) => id !== pane) : [...tab.broadcastPaneIds, pane],
      broadcastArmedPaneIds: [],
    }),
    // Arms only a set that is still exactly the current selection.
    armBroadcast: (id, panes) => changeTab(id, (tab) => tab.broadcastEnabled && panes.length === tab.broadcastPaneIds.length
      && panes.every((pane) => tab.broadcastPaneIds.includes(pane)) ? { ...tab, broadcastArmedPaneIds: [...panes] } : tab),
    setPaneTheme: (id, pane, theme) => changeTab(id, (tab) => {
      if (!findPane(tab.root, pane) || tab.paneStates[pane]?.themeOverride === theme) return tab;
      return { ...tab, paneStates: { ...tab.paneStates, [pane]: { ...tab.paneStates[pane], themeOverride: theme } } };
    }),
    moveTab: (id, target) => set((state) => {
      const from = state.tabs.findIndex((tab) => tab.id === id), to = state.tabs.findIndex((tab) => tab.id === target);
      if (from < 0 || to < 0 || from === to) return state;
      const tabs = [...state.tabs];
      tabs.splice(to, 0, ...tabs.splice(from, 1));
      return { tabs };
    }),
    closeTerminal: (id) => set((state) => closedTab(state, id)),
    reopenTerminal: () => set((state) => {
      const closed = state.closedTabs.at(-1);
      if (!closed) return state;
      const root = renewed(closed.root), leaves = paneLeaves(root);
      const focused = leaves[Math.min(closed.focusedIndex, leaves.length - 1)];
      const paneStates: Record<string, PaneState> = {};
      leaves.forEach((pane, index) => { const theme = closed.themes[index]; if (theme) paneStates[pane.paneId] = { themeOverride: theme }; });
      const reopened: TerminalTab = {
        id: ("tab-" + state.nextId) as TabId, label: closed.label, launch: focused.launch, root, focusedPaneId: focused.paneId, paneStates,
        broadcastEnabled: false, broadcastPaneIds: [], broadcastArmedPaneIds: [],
      };
      const tabs = [...state.tabs];
      tabs.splice(Math.min(closed.index, tabs.length), 0, reopened);
      return { tabs, activeTab: reopened.id, nextId: state.nextId + 1, closedTabs: state.closedTabs.slice(0, -1) };
    }),
    selectTab: (id) => set((state) => state.tabs.some((tab) => tab.id === id) ? { activeTab: id } : state),
    updateSession: (id, snapshot, paneId, launch) => changeTab(id, (tab) => {
      const target = paneId ?? tab.focusedPaneId, pane = findPane(tab.root, target), current = tab.paneStates[target];
      if (!pane || (launch && pane.launch !== launch) || (current?.session?.id === snapshot.id && current.session.state === snapshot.state
        && current.session.exit_code === snapshot.exit_code && current.session.error_code === snapshot.error_code && !current.failed)) return tab;
      return { ...tab, paneStates: { ...tab.paneStates, [target]: {
        ...(current?.themeOverride ? { themeOverride: current.themeOverride } : {}),
        session: { id: snapshot.id, state: snapshot.state, exit_code: snapshot.exit_code, error_code: snapshot.error_code },
      } } };
    }),
    failTab: (id, paneId, launch) => changeTab(id, (tab) => {
      const target = paneId ?? tab.focusedPaneId, pane = findPane(tab.root, target);
      return !pane || (launch && pane.launch !== launch) || tab.paneStates[target]?.failed ? tab : {
        ...tab, paneStates: { ...tab.paneStates, [target]: { ...tab.paneStates[target], failed: true } },
      };
    }),
  };
});
