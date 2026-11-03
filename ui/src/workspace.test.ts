import { beforeEach, describe, expect, it } from "vitest";
import { tabStatus, useWorkspace } from "./workspace";
import type { HostId, TerminalSessionId, TerminalSnapshot } from "./terminal/contracts";
import type { PaneId } from "./panes/contracts";
import { findPane, paneLeaves, splitId } from "./panes/tree";

const metrics: TerminalSnapshot["metrics"] = {
  queued_bytes: 0, in_flight_bytes: 0, max_outstanding_bytes: 0, input_bytes: 0, output_bytes: 0, frame_count: 0,
  max_pending_bytes: 0, max_in_flight_bytes: 0, pause_count: 0, resume_count: 0, canceled: false, channel_closed: false,
};
const snapshot = (state: TerminalSnapshot["state"]): TerminalSnapshot => ({
  id: "session-1" as TerminalSessionId, state, exit_code: state === "exited" ? 0 : null, error_code: null, metrics,
});
const sshTab = () => {
  useWorkspace.getState().addSshTerminal("host-1" as HostId, "build-01", "graphite");
  const state = useWorkspace.getState();
  return state.tabs.find((tab) => tab.id === state.activeTab)!;
};

describe("tab signal state", () => {
  beforeEach(() => {
    const state = useWorkspace.getState();
    for (const tab of state.tabs) state.closeTerminal(tab.id);
  });

  it("follows an SSH session from connecting to live to ended", () => {
    const tab = sshTab();
    expect(tabStatus(tab)).toBe("connecting");
    useWorkspace.getState().updateSession(tab.id, snapshot("running"));
    expect(tabStatus(useWorkspace.getState().tabs.find((item) => item.id === tab.id)!)).toBe("live");
    useWorkspace.getState().updateSession(tab.id, snapshot("exited"));
    expect(tabStatus(useWorkspace.getState().tabs.find((item) => item.id === tab.id)!)).toBe("ended");
  });

  it("stops reporting a connection attempt once the start failed", () => {
    const tab = sshTab();
    useWorkspace.getState().failTab(tab.id);
    expect(tabStatus(useWorkspace.getState().tabs.find((item) => item.id === tab.id)!)).toBe("failed");
  });

  it("describes a local shell that has not reported yet as starting", () => {
    const local = useWorkspace.getState().tabs[0];
    expect(local.launch.kind).toBe("local");
    expect(tabStatus(local)).toBe("starting");
    expect(tabStatus({ ...local, paneStates: { [local.focusedPaneId]: { session: { ...snapshot("error") } } } })).toBe("failed");
  });

  it("focuses a host's existing tab instead of opening a duplicate", () => {
    const tab = sshTab();
    useWorkspace.getState().updateSession(tab.id, snapshot("running"));
    useWorkspace.getState().addTerminal();
    useWorkspace.getState().openSshHost("host-1" as HostId, "build-01", "graphite");
    const state = useWorkspace.getState();
    expect(state.activeTab).toBe(tab.id);
    expect(state.tabs.filter((item) => item.launch.kind === "ssh")).toHaveLength(1);
    useWorkspace.getState().openSshHost("host-1" as HostId, "build-01", "graphite", true);
    expect(useWorkspace.getState().tabs.filter((item) => item.launch.kind === "ssh")).toHaveLength(2);
  });

  it("reconnects an ended host tab in place with a new attempt", () => {
    const tab = sshTab();
    useWorkspace.getState().updateSession(tab.id, snapshot("exited"));
    const before = useWorkspace.getState().tabs.find((item) => item.id === tab.id)!;
    useWorkspace.getState().addTerminal();
    useWorkspace.getState().openSshHost("host-1" as HostId, "build-01", "light");
    const after = useWorkspace.getState().tabs.find((item) => item.id === tab.id)!;
    expect(useWorkspace.getState().activeTab).toBe(tab.id);
    expect(tabStatus(after)).toBe("connecting");
    expect(after.launch).not.toBe(before.launch);
    expect(after.launch.kind === "ssh" && before.launch.kind === "ssh" && after.launch.attempt_id !== before.launch.attempt_id).toBe(true);
    expect(after.launch.kind === "ssh" && after.launch.theme).toBe("light");
  });

  it("restarts a local shell without changing its tab identity", () => {
    const local = useWorkspace.getState().tabs[0];
    useWorkspace.getState().failTab(local.id);
    useWorkspace.getState().restartTab(local.id);
    const after = useWorkspace.getState().tabs.find((item) => item.id === local.id)!;
    expect(tabStatus(after)).toBe("starting");
    expect(after.launch).not.toBe(local.launch);
  });
});

describe("pane workspace state", () => {
  beforeEach(() => {
    useWorkspace.getState().addTerminal({ shell_id: "wsl:Ubuntu", working_directory: "/home/dev" }, "Ubuntu");
    const active = useWorkspace.getState().activeTab;
    for (const tab of useWorkspace.getState().tabs) if (tab.id !== active) useWorkspace.getState().closeTerminal(tab.id);
  });
  const active = () => useWorkspace.getState().tabs.find((tab) => tab.id === useWorkspace.getState().activeTab)!;

  it("keeps stable pane identities and existing launches through nested splits and resizes", () => {
    const original = active(), id = original.focusedPaneId, launch = original.launch;
    useWorkspace.getState().updateSession(original.id, snapshot("running"), id, launch);
    useWorkspace.getState().splitPane(original.id, id, "horizontal");
    const second = active().focusedPaneId;
    useWorkspace.getState().splitPane(original.id, second, "vertical");
    const tab = active();
    expect(paneLeaves(tab.root)).toHaveLength(3);
    expect(findPane(tab.root, id)?.launch).toBe(launch);
    expect(paneLeaves(tab.root).map((pane) => pane.launch)).toEqual(Array(3).fill(launch));
    expect(tab.paneStates[id]?.session?.id).toBe("session-1");
    expect(tab.root.kind === "split" && tab.root.second.kind).toBe("split");
    if (tab.root.kind !== "split") throw new Error("expected split");
    useWorkspace.getState().resizePaneSplit(tab.id, splitId(tab.root), 0.7);
    expect(active().root.kind === "split" && active().root).toMatchObject({ ratio: 0.7 });
    expect(findPane(active().root, id)?.launch).toBe(launch);
    useWorkspace.getState().resizePaneSplit(tab.id, splitId(tab.root), NaN);
    expect(active().root).toMatchObject({ ratio: 0.7 });
  });

  it("creates a unique SSH attempt for every new pane on the same host", () => {
    const tab = sshTab();
    useWorkspace.getState().splitPane(tab.id, tab.focusedPaneId, "vertical");
    const leaves = paneLeaves(active().root);
    expect(leaves[0].launch).toBe(tab.launch);
    expect(leaves[1].launch).toMatchObject({ kind: "ssh", host_id: "host-1", theme: "graphite" });
    expect(leaves[1].launch).not.toEqual(leaves[0].launch);
    expect(new Set(leaves.map((pane) => pane.paneId)).size).toBe(2);
  });

  it("keeps a palette theme override through session updates, splits, and pane restarts", () => {
    const tab = sshTab(), first = tab.focusedPaneId;
    useWorkspace.getState().setPaneTheme(tab.id, first, "light");
    useWorkspace.getState().updateSession(tab.id, snapshot("running"), first, tab.launch);
    expect(active().paneStates[first]).toMatchObject({ themeOverride: "light", session: { state: "running" } });
    useWorkspace.getState().failTab(tab.id, first, tab.launch);
    expect(active().paneStates[first]?.failed).toBe(true);
    useWorkspace.getState().updateSession(tab.id, snapshot("running"), first, tab.launch);
    expect(active().paneStates[first]?.failed).toBeUndefined();

    useWorkspace.getState().splitPane(tab.id, first, "horizontal");
    const second = active().focusedPaneId;
    expect(active().paneStates[second]?.themeOverride).toBe("light");
    useWorkspace.getState().restartPane(tab.id, first);
    expect(active().paneStates[first]?.themeOverride).toBe("light");
    expect(findPane(active().root, first)?.launch).toMatchObject({ kind: "ssh", theme: "graphite" });
  });

  it("closes only one pane, collapses its branch, and removes its broadcast membership", () => {
    const tab = active(), first = tab.focusedPaneId;
    useWorkspace.getState().updateSession(tab.id, snapshot("running"));
    useWorkspace.getState().splitPane(tab.id, first, "horizontal");
    const second = active().focusedPaneId;
    useWorkspace.getState().splitPane(tab.id, second, "vertical");
    const third = active().focusedPaneId;
    useWorkspace.getState().toggleBroadcastPane(tab.id, second);
    useWorkspace.getState().toggleBroadcastPane(tab.id, third);
    useWorkspace.getState().setBroadcast(tab.id, true);
    useWorkspace.getState().closePane(tab.id, second);
    expect(paneLeaves(active().root).map((pane) => pane.paneId)).toEqual([first, third]);
    expect(active().focusedPaneId).toBe(third);
    expect(active().broadcastPaneIds).toEqual([third]);
    expect(active().paneStates[first]?.session?.state).toBe("running");
    useWorkspace.getState().closePane(tab.id, third);
    expect(active().root).toEqual(tab.root);
    expect(active().focusedPaneId).toBe(first);
    useWorkspace.getState().closePane(tab.id, first);
    expect(useWorkspace.getState().tabs).toHaveLength(1);
    expect(active().id).not.toBe(tab.id);
  });

  it("isolates final states and ignores late updates from closed or restarted launches", () => {
    const tab = active(), first = tab.focusedPaneId;
    useWorkspace.getState().splitPane(tab.id, first, "horizontal");
    const second = active().focusedPaneId, secondLaunch = active().launch;
    useWorkspace.getState().updateSession(tab.id, snapshot("running"), first, tab.launch);
    useWorkspace.getState().failTab(tab.id, second, secondLaunch);
    expect(tabStatus(active())).toBe("live");
    useWorkspace.getState().restartPane(tab.id, second);
    useWorkspace.getState().failTab(tab.id, second, secondLaunch);
    useWorkspace.getState().updateSession(tab.id, snapshot("exited"), second, secondLaunch);
    expect(active().paneStates[second]).toBeUndefined();
    useWorkspace.getState().closePane(tab.id, second);
    const after = active();
    useWorkspace.getState().updateSession(tab.id, snapshot("running"), second);
    useWorkspace.getState().failTab(tab.id, second);
    useWorkspace.getState().focusPane(tab.id, "missing" as PaneId);
    expect(active()).toBe(after);
  });

  it("reorders tabs without replacing pane trees, sessions, focus or broadcast selection", () => {
    const first = active();
    useWorkspace.getState().toggleBroadcastPane(first.id, first.focusedPaneId);
    const selected = active();
    useWorkspace.getState().addTerminal();
    const second = active();
    useWorkspace.getState().moveTab(second.id, first.id);
    expect(useWorkspace.getState().tabs).toEqual([second, selected]);
    expect(useWorkspace.getState().activeTab).toBe(second.id);
    expect(useWorkspace.getState().tabs[1]).toBe(selected);
  });
});
