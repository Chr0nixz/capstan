import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-editor.json";
import { RemoteEditors } from "./RemoteEditors";
import { remoteEditors } from "./controller";
import type { RetryBinding } from "./transport";
import { decodeRemoteEditSnapshot } from "../sftp/contracts";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../terminal/bridge", () => ({ isTauriAvailable: () => true, invokeTauri: (command: string, args?: unknown) => mocks.invoke(command, args) }));
let root: Root, container: HTMLDivElement;
const button = (name: string): HTMLButtonElement | undefined => [...document.querySelectorAll<HTMLButtonElement>("button")].find((entry) => entry.textContent === name);
beforeEach(async () => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation(async (command: string) => command === "remote_edit_snapshot" ? fixture : false);
  remoteEditors.store.setState({ snapshot: null, error: null, pending: [] });
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  await act(async () => root.render(<RemoteEditors locale="en" theme="graphite" hosts={{}} />));
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
describe("global remote editor entry", () => {
  it("waits for a new matching host session before sending an explicitly requested retry", async () => {
    const interrupted = { ...fixture, sessions: [{ ...fixture.sessions[0], state: "residual", error: "connection_lost" }, fixture.sessions[1]] };
    mocks.invoke.mockImplementation(async (command: string) => command === "retry_remote_edit" ? interrupted.sessions[0] : interrupted);
    await act(async () => remoteEditors.store.setState({ snapshot: decodeRemoteEditSnapshot(interrupted) }));
    const reconnect = vi.fn();
    await act(async () => root.render(<RemoteEditors locale="en" theme="graphite" hosts={{}} onRetrySessionRequested={reconnect} />));
    await act(async () => button("Retry upload")?.click());
    expect(reconnect).toHaveBeenCalledWith(fixture.sessions[0].hostId);
    expect(mocks.invoke.mock.calls.some(([command]) => command === "retry_remote_edit")).toBe(false);
    expect(container.textContent).toContain("Connect this SSH host again");
    const binding = { terminal: "new-terminal", session: "00000000-0000-4000-8000-000000000041", hostId: fixture.sessions[0].hostId } as RetryBinding;
    await act(async () => root.render(<RemoteEditors locale="en" theme="graphite" hosts={{}} sessions={[binding]} onRetrySessionRequested={reconnect} />));
    expect(mocks.invoke).toHaveBeenCalledWith("retry_remote_edit", { id: fixture.sessions[0].id, terminal: binding.terminal, session: binding.session });
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "retry_remote_edit")).toHaveLength(1);
  });
  it("shows retained remote paths and sends cleanup with only the edit ID", async () => {
    expect(container.textContent).toContain(".capstan-edit-example.partial");
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "cleanup_remote_edit_temporaries") throw { code: "editor_residual" };
      return fixture;
    });
    await act(async () => button("Retry remote cleanup")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("cleanup_remote_edit_temporaries", { id: fixture.sessions[0].id, terminal: null, session: null });
    expect(container.textContent).toContain(".capstan-edit-example.partial");
  });
  it("rebinds a recovered cleanup request without requesting an upload", async () => {
    const recovered = { ...fixture, sessions: [{ ...fixture.sessions[0], recovered: true, remotePath: null, state: "residual", error: null }] };
    mocks.invoke.mockImplementation(async (command: string) => command === "cleanup_remote_edit_temporaries" ? recovered.sessions[0] : recovered);
    await act(async () => remoteEditors.store.setState({ snapshot: decodeRemoteEditSnapshot(recovered) }));
    const reconnect = vi.fn();
    await act(async () => root.render(<RemoteEditors locale="en" theme="graphite" hosts={{}} onRetrySessionRequested={reconnect} />));
    await act(async () => button("Retry remote cleanup")?.click());
    expect(reconnect).toHaveBeenCalledWith(fixture.sessions[0].hostId);
    expect(container.textContent).toContain("A recovered document will not be uploaded");
    expect(button("Retry upload")).toBeUndefined();
    const binding = { terminal: "new-terminal", session: "00000000-0000-4000-8000-000000000041", hostId: fixture.sessions[0].hostId } as RetryBinding;
    await act(async () => root.render(<RemoteEditors locale="en" theme="graphite" hosts={{}} sessions={[binding]} onRetrySessionRequested={reconnect} />));
    expect(mocks.invoke).toHaveBeenCalledWith("cleanup_remote_edit_temporaries", { id: recovered.sessions[0].id, terminal: binding.terminal, session: binding.session });
    expect(mocks.invoke.mock.calls.some(([command]) => command === "retry_remote_edit" || command === "resolve_remote_edit_conflict")).toBe(false);
  });
  it("allows explicitly keeping the recovery files and shows unreadable-metadata recovery", async () => {
    const recovered = { ...fixture, sessions: [{ ...fixture.sessions[1], error: "editor_residual" }] };
    mocks.invoke.mockImplementation(async (command: string) => command === "close_remote_edit" ? true : { ...recovered, sessions: [] });
    await act(async () => remoteEditors.store.setState({ snapshot: decodeRemoteEditSnapshot(recovered) }));
    expect(container.textContent).toContain("Recovery metadata could not be verified");
    await act(async () => button("End editing…")?.click());
    await act(async () => button("Keep files for next start")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("close_remote_edit", { id: fixture.sessions[1].id, action: "keep_for_recovery", locale: "en" });
    expect(mocks.invoke.mock.calls.some(([command, args]) => command === "close_remote_edit" && (args as { action: string }).action === "discard")).toBe(false);
    expect(document.querySelector('[role="dialog"]')).toBeNull();
  });
  it("retains recovered copies when the native export picker is canceled", async () => {
    expect(container.textContent).toContain("Recovered local copy");
    const closeButtons = [...container.querySelectorAll<HTMLButtonElement>("button")].filter((entry) => entry.textContent === "End editing…");
    await act(async () => closeButtons[1].click());
    await act(async () => button("Export local copy…")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("close_remote_edit", { id: fixture.sessions[1].id, action: "save_local_copy", locale: "en" });
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
    expect(container.textContent).toContain("recovered.txt");
  });
  it("keeps a conflict visible after cleanup fails and uses explicit discard wording", async () => {
    await act(async () => button("Review conflict")?.click());
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "resolve_remote_edit_conflict") throw { code: "editor_residual" };
      return fixture;
    });
    await act(async () => button("Keep remote and discard local")?.click());
    expect(document.querySelector('[role="dialog"]')).not.toBeNull();
    expect(document.body.textContent).toContain("Cleanup failures retain the session");
  });
  it("renders recovery and conflict decisions in Chinese and preserves the row on cancel", async () => {
    await act(async () => root.render(<RemoteEditors locale="zh-CN" theme="light" hosts={{}} />));
    expect(container.textContent).toContain("恢复的本地副本");
    await act(async () => button("处理冲突")?.click());
    expect(button("保留远端并放弃本地")).toBeDefined();
    await act(async () => button("继续编辑")?.click());
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(container.textContent).toContain("notes 中文.txt");
    expect(mocks.invoke.mock.calls.some(([command]) => command === "close_remote_edit" || command === "resolve_remote_edit_conflict")).toBe(false);
  });
});
