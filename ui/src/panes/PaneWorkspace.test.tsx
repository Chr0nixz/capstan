import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PaneTools, PaneWorkspace } from "./PaneWorkspace";
import { copy } from "../i18n";
import { useWorkspace } from "../workspace";
import { paneLeaves, splitId } from "./tree";
import type { TerminalSession, TerminalSessionOptions } from "../terminal/session";
import type { ControllerCallbacks } from "../terminal/controller";
import type { TerminalLaunch } from "../terminal/contracts";

const mocks = vi.hoisted(() => ({
  sessions: [] as { options: TerminalSessionOptions; session: TerminalSession; host: HTMLElement }[],
  controllers: [] as { launch: TerminalLaunch; callbacks: ControllerCallbacks; sendInput: ReturnType<typeof vi.fn<(bytes: Uint8Array) => Promise<void>>>; dispose: ReturnType<typeof vi.fn> }[],
}));
vi.mock("../terminal/bridge", () => ({ isTauriAvailable: () => true }));
vi.mock("../terminal/session", () => ({
  createTerminalSession: (host: HTMLElement, _visible: boolean, options: TerminalSessionOptions) => {
    const session = {
      write: vi.fn(), writeLocal: vi.fn(), setVisible: vi.fn(), setTheme: vi.fn(), focus: vi.fn(), flushInput: vi.fn(() => []), stopInput: vi.fn(), resumeInput: vi.fn(),
      search: vi.fn(), clearSearch: vi.fn(), settleOutput: vi.fn(async () => undefined), dispose: vi.fn(), rendererMounted: true, disposed: false,
    };
    mocks.sessions.push({ host, options, session });
    return session;
  },
}));
vi.mock("../connections/controller", () => ({
  createRecoveringController: (launch: TerminalLaunch, callbacks: ControllerCallbacks) => {
    const controller = { launch, callbacks, sendInput: vi.fn(async (_bytes: Uint8Array) => undefined), dispose: vi.fn(async () => undefined),
      acknowledge: vi.fn(), resize: vi.fn(), ended: false };
    mocks.controllers.push(controller);
    return controller;
  },
}));

let root: Root, container: HTMLDivElement;
function Harness() {
  const state = useWorkspace();
  const current = state.tabs.find((tab) => tab.id === state.activeTab)!;
  const [focusRequest, requestFocus] = [0, () => undefined];
  return <>
    <PaneTools tab={current} labels={copy.en} onRequestFocus={requestFocus} />
    {state.tabs.map((tab) => <div key={tab.id} hidden={tab.id !== state.activeTab}>
      <PaneWorkspace tab={tab} visible={tab.id === state.activeTab} focusRequest={focusRequest} labels={copy.en} name={tab.label ?? "Local"} productionHosts={new Set()}
        onRequestFocus={requestFocus} />
    </div>)}
  </>;
}
const current = () => useWorkspace.getState().tabs.find((tab) => tab.id === useWorkspace.getState().activeTab)!;
const encode = (text: string) => new TextEncoder().encode(text);
const writes = () => mocks.controllers.map((controller) => controller.sendInput.mock.calls.length);
async function split(): Promise<void> {
  await act(async () => { const tab = current(); useWorkspace.getState().splitPane(tab.id, tab.focusedPaneId, "horizontal"); });
}
async function broadcast(): Promise<void> {
  await act(async () => {
    const tab = current();
    for (const pane of paneLeaves(tab.root)) useWorkspace.getState().toggleBroadcastPane(tab.id, pane.paneId);
    useWorkspace.getState().setBroadcast(tab.id, true);
  });
}

beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  useWorkspace.getState().addTerminal();
  const id = useWorkspace.getState().activeTab;
  for (const tab of useWorkspace.getState().tabs) if (tab.id !== id) useWorkspace.getState().closeTerminal(tab.id);
  mocks.sessions.length = 0; mocks.controllers.length = 0;
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  await act(async () => root.render(<Harness />));
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove(); vi.restoreAllMocks();
});

describe("pane workspace integration", () => {
  it("keeps every surviving xterm and controller through nesting, resize, tab reorder and branch collapse", async () => {
    const first = current(), originalHost = mocks.sessions[0].host;
    await split();
    await act(async () => { const tab = current(); useWorkspace.getState().splitPane(tab.id, tab.focusedPaneId, "vertical"); });
    expect(mocks.sessions).toHaveLength(3);
    expect(mocks.sessions[0].host).toBe(originalHost);
    const tree = current().root;
    if (tree.kind !== "split") throw new Error("expected split");
    await act(async () => useWorkspace.getState().resizePaneSplit(first.id, splitId(tree), 0.65));
    await act(async () => useWorkspace.getState().addTerminal());
    const otherTab = current();
    expect(mocks.sessions).toHaveLength(4);
    expect(mocks.sessions.slice(0, 3).every(({ session }) => vi.mocked(session.setVisible).mock.lastCall?.[0] === false)).toBe(true);
    await act(async () => { useWorkspace.getState().moveTab(otherTab.id, first.id); useWorkspace.getState().selectTab(first.id); });
    expect(mocks.sessions.slice(0, 3).every(({ session }) => vi.mocked(session.setVisible).mock.lastCall?.[0] === true)).toBe(true);
    const second = paneLeaves(current().root)[1].paneId;
    await act(async () => useWorkspace.getState().closePane(first.id, second));
    expect(mocks.sessions).toHaveLength(4);
    expect(mocks.controllers.map((controller) => controller.dispose.mock.calls.length)).toEqual([0, 1, 0, 0]);
    expect(mocks.sessions[0].host).toBe(originalHost);
    expect(container.contains(originalHost)).toBe(true);
  });

  it("isolates ordinary input and keeps background terminal replies scoped to their source", async () => {
    await split();
    await act(async () => {
      await mocks.sessions[0].options.sendProtocolInput?.(encode("\x1b[0n"));
      await mocks.sessions[1].options.sendInput?.(encode("中文"));
    });
    expect(writes()).toEqual([1, 1]);
    const first = paneLeaves(current().root)[0].paneId;
    await act(async () => useWorkspace.getState().focusPane(current().id, first));
    await act(async () => { await mocks.sessions[0].options.sendInput?.(encode("first")); });
    expect(writes()).toEqual([2, 1]);
  });

  it("blocks empty broadcast selection and frames only the selected panes", async () => {
    await split();
    await act(async () => useWorkspace.getState().setBroadcast(current().id, true));
    expect(container.querySelector(".broadcast-status")?.textContent).toContain(copy.en.broadcastNoTargets);
    await act(async () => { await mocks.sessions[1].options.sendInput?.(encode("no target")); });
    expect(writes()).toEqual([0, 0]);
    const checkbox = container.querySelector<HTMLInputElement>("input[type='checkbox']")!;
    await act(async () => checkbox.click());
    expect(container.querySelectorAll(".workspace-pane[data-broadcast]")).toHaveLength(1);
    expect(current().broadcastPaneIds).toHaveLength(1);
  });

  it("confirms a broadcast once before any write, starts on Cancel, and cancels every target", async () => {
    await split(); await broadcast();
    const confirm = vi.spyOn(window, "confirm");
    expect(mocks.sessions[1].options.confirmMultilinePaste?.(2)).toBe(true);
    expect(confirm).not.toHaveBeenCalled();
    let task: Promise<void> | void;
    await act(async () => { task = mocks.sessions[1].options.sendInput?.(encode("\x1b[200~one\rtwo\x1b[201~")); });
    expect(writes()).toEqual([0, 0]);
    const dialog = document.body.querySelector("[role='alertdialog']");
    expect(dialog?.textContent).toContain("Local · Pane 1");
    expect(dialog?.textContent).toContain("Local · Pane 2");
    expect(document.activeElement?.textContent).toBe("Cancel");
    await act(async () => { [...dialog!.querySelectorAll("button")].find((button) => button.textContent === "Cancel")!.click(); await task; });
    expect(writes()).toEqual([0, 0]);
    expect(document.body.querySelector("[role='alertdialog']")).toBeNull();
  });

  it("asks once per target set, then lets single commands through until the selection changes", async () => {
    await split(); await broadcast();
    const dialog = () => document.body.querySelector("[role='alertdialog']");
    // Wrapped so awaiting the send does not also await the pending confirmation.
    const send = async (text: string): Promise<{ task: Promise<void> | void }> => {
      let task: Promise<void> | void;
      await act(async () => { task = mocks.sessions[1].options.sendInput?.(encode(text)); });
      return { task: task! };
    };
    const answer = async (label: string, { task }: { task: Promise<void> | void }): Promise<void> => act(async () => {
      [...dialog()!.querySelectorAll("button")].find((button) => button.textContent === label)!.click();
      await task;
    });
    let task = await send("date\r");
    expect(dialog()?.textContent).toContain(copy.en.confirmBroadcastArmTitle(2));
    expect(writes()).toEqual([0, 0]);
    await answer(copy.en.confirmBroadcastSend(2), task);
    expect(writes()).toEqual([1, 1]);
    expect(current().broadcastArmedPaneIds).toHaveLength(2);
    expect(container.querySelector(".broadcast-status")?.textContent).toContain(copy.en.broadcastArmed);
    await act(async () => { await mocks.sessions[1].options.sendInput?.(encode("uptime\r")); });
    expect(dialog()).toBeNull();
    expect(writes()).toEqual([2, 2]);
    task = await send("\x1b[200~ls\rpwd\x1b[201~");
    expect(dialog()?.textContent).toContain(copy.en.confirmBroadcastTitle);
    await answer("Cancel", task);
    expect(writes()).toEqual([2, 2]);
    expect(current().broadcastArmedPaneIds).toHaveLength(2);
    await act(async () => { const tab = current(); useWorkspace.getState().toggleBroadcastPane(tab.id, paneLeaves(tab.root)[0].paneId); });
    expect(current().broadcastArmedPaneIds).toHaveLength(0);
    task = await send("\r");
    expect(dialog()?.textContent).toContain(copy.en.confirmBroadcastArmTitle(1));
    await answer("Cancel", task);
    expect(writes()).toEqual([2, 2]);
  });

  it("reports partial/uncertain failures by pane and never replays a successful destination", async () => {
    await split(); await broadcast();
    mocks.controllers[0].sendInput.mockRejectedValueOnce(new Error("server-private-data"));
    let task: Promise<void> | void;
    await act(async () => { task = mocks.sessions[1].options.sendInput?.(encode("date\r")); });
    expect(writes()).toEqual([0, 0]);
    const send = [...document.body.querySelectorAll("[role='alertdialog'] button")].find((button) => button.textContent === "Send to 2 panes") as HTMLButtonElement;
    await act(async () => { send.click(); await task; });
    expect(writes()).toEqual([1, 1]);
    expect(container.querySelector(".pane-input-error")?.textContent).toContain("Local · Pane 1 (delivery uncertain)");
    expect(container.textContent).not.toContain("server-private-data");
    expect(mocks.controllers[0].dispose).toHaveBeenCalledTimes(1);
    expect(mocks.controllers[1].dispose).not.toHaveBeenCalled();
    expect(container.querySelector(".workspace-pane .session-ended")?.textContent).toContain("Restart shell");
    await act(async () => { await Promise.resolve(); });
    expect(writes()).toEqual([1, 1]);
  });

  it("cancels pending broadcast when switching tabs, without sending into hidden panes", async () => {
    await split(); await broadcast();
    let task: Promise<void> | void;
    await act(async () => { task = mocks.sessions[1].options.sendInput?.(encode("x\n")); });
    expect(document.body.querySelector("[role='alertdialog']")).not.toBeNull();
    await act(async () => { useWorkspace.getState().addTerminal(); await task; });
    expect(document.body.querySelector("[role='alertdialog']")).toBeNull();
    expect(writes()).toEqual([0, 0, 0]);
  });

  it("keeps closed target names accurate and does not send old confirmed bytes to a surviving sibling", async () => {
    await split();
    const first = paneLeaves(current().root)[0].paneId;
    await act(async () => {
      useWorkspace.getState().toggleBroadcastPane(current().id, first);
      useWorkspace.getState().setBroadcast(current().id, true);
    });
    let task: Promise<void> | void;
    await act(async () => { task = mocks.sessions[1].options.sendInput?.(encode("x\r")); });
    await act(async () => useWorkspace.getState().closePane(current().id, first));
    const send = [...document.body.querySelectorAll("[role='alertdialog'] button")].find((button) => button.textContent === "Send to 1 pane") as HTMLButtonElement;
    await act(async () => { send.click(); await task; });
    expect(writes()).toEqual([0, 0]);
    expect(container.querySelector(".pane-input-error")?.textContent).toContain("Local · Pane 1 (session closed)");
    expect(container.querySelector(".workspace-pane-heading")?.textContent).toContain("Local · Pane 2");
  });

  it("does not broadcast protocol replies generated by a background terminal", async () => {
    await split();
    const tab = current();
    await act(async () => {
      useWorkspace.getState().toggleBroadcastPane(tab.id, tab.focusedPaneId);
      useWorkspace.getState().setBroadcast(tab.id, true);
      await mocks.sessions[0].options.sendProtocolInput?.(encode("\x1b[1;1R"));
    });
    expect(writes()).toEqual([1, 0]);
    expect(mocks.controllers[0].sendInput).toHaveBeenCalledExactlyOnceWith(encode("\x1b[1;1R"));
  });

  it.each(["both", "other", "empty", "disabled"])("keeps focused replies in their source with %s broadcast selection", async (selection) => {
    await split();
    const tab = current(), first = paneLeaves(tab.root)[0].paneId;
    await act(async () => {
      if (selection === "both" || selection === "other") useWorkspace.getState().toggleBroadcastPane(tab.id, first);
      if (selection === "both") useWorkspace.getState().toggleBroadcastPane(tab.id, tab.focusedPaneId);
      useWorkspace.getState().setBroadcast(tab.id, selection !== "disabled");
      await mocks.sessions[1].options.sendProtocolInput?.(encode("\x1b[7;18R"));
    });
    expect(writes()).toEqual([0, 1]);
    expect(mocks.controllers[1].sendInput).toHaveBeenCalledExactlyOnceWith(encode("\x1b[7;18R"));
    expect(document.body.querySelector("[role='alertdialog']")).toBeNull();
  });

  it("routes protocol replies during confirmation and cancellation without releasing user bytes", async () => {
    await split(); await broadcast();
    let task: Promise<void> | void;
    await act(async () => { task = mocks.sessions[1].options.sendInput?.(encode("user\r")); });
    await act(async () => { await mocks.sessions[1].options.sendProtocolInput?.(encode("\x1b[7;18R")); });
    expect(writes()).toEqual([0, 1]);
    const dialog = document.body.querySelector("[role='alertdialog']")!;
    await act(async () => { [...dialog.querySelectorAll("button")].find((button) => button.textContent === "Cancel")!.click(); await task; });
    expect(writes()).toEqual([0, 1]);
    expect(mocks.controllers[1].sendInput).toHaveBeenCalledExactlyOnceWith(encode("\x1b[7;18R"));
  });

  it("treats user bytes identical to a DSR response as user input without string filtering", async () => {
    await split();
    const tab = current(), first = paneLeaves(tab.root)[0].paneId;
    await act(async () => {
      useWorkspace.getState().toggleBroadcastPane(tab.id, first);
      useWorkspace.getState().setBroadcast(tab.id, true);
      await mocks.sessions[1].options.sendInput?.(encode("\x1b[7;18R"));
    });
    expect(writes()).toEqual([1, 0]);
  });

  it("cancels a confirmed broadcast whose source closes before destination acceptance", async () => {
    await split(); await broadcast();
    let task: Promise<void> | void;
    await act(async () => { task = mocks.sessions[1].options.sendInput?.(encode("user\r")); });
    await act(async () => useWorkspace.getState().closePane(current().id, current().focusedPaneId));
    const dialog = document.body.querySelector("[role='alertdialog']")!;
    await act(async () => { [...dialog.querySelectorAll("button")].find((button) => button.textContent === "Send to 2 panes")!.click(); await task; });
    expect(writes()).toEqual([0, 0]);
  });

  it("keeps data already accepted by a destination FIFO when its source closes", async () => {
    await split();
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    mocks.controllers[0].sendInput.mockImplementationOnce(() => gate);
    const holding = mocks.sessions[0].options.sendInput?.(encode("hold"));
    await act(async () => {
      const tab = current(), first = paneLeaves(tab.root)[0].paneId;
      useWorkspace.getState().toggleBroadcastPane(tab.id, first);
      useWorkspace.getState().setBroadcast(tab.id, true);
    });
    const accepted = mocks.sessions[1].options.sendInput?.(encode("accepted"));
    expect(writes()).toEqual([1, 0]);
    await act(async () => useWorkspace.getState().closePane(current().id, current().focusedPaneId));
    await act(async () => { release(); await holding; await accepted; });
    expect(mocks.controllers[0].sendInput.mock.calls.map(([value]) => new TextDecoder().decode(value))).toEqual(["hold", "accepted"]);
  });

  it("keeps protocol replies behind data already accepted by their own destination FIFO", async () => {
    await split();
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    mocks.controllers[1].sendInput.mockImplementationOnce(() => gate);
    const holding = mocks.sessions[1].options.sendInput?.(encode("hold"));
    const reply = mocks.sessions[1].options.sendProtocolInput?.(encode("\x1b[7;18R"));
    expect(writes()).toEqual([0, 1]);
    await act(async () => { release(); await holding; await reply; });
    expect(mocks.controllers[1].sendInput.mock.calls.map(([value]) => new TextDecoder().decode(value))).toEqual(["hold", "\x1b[7;18R"]);
  });
});
