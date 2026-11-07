import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M4-contract.json";
import type { TerminalSessionId } from "../terminal/contracts";
import { useServerStatus } from "./useServerStatus";

Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });

it("reports the focused SSH connection ID and clears stale generations and local tabs", async () => {
  const first = "first" as TerminalSessionId;
  const replacement = "replacement" as TerminalSessionId;
  const status = fixture.serverStatus;
  const next = { ...status, generation: (BigInt(status.generation) + 1n).toString(), cpuPercent: 77 };
  const listeners = new Set<(event: { payload: unknown }) => void>();
  const emit = (payload: unknown) => listeners.forEach((receive) => receive({ payload }));
  const invoke = vi.fn(async (command: string, args?: Record<string, unknown>) => {
    if (command !== "server_status_binding") return;
    return args?.terminal === first ? status : next;
  });
  window.__TAURI__ = {
    core: { Channel: class { onmessage = null; }, invoke },
    event: { listen: async (_name: string, callback: (event: { payload: unknown }) => void) => {
      listeners.add(callback);
      return () => { listeners.delete(callback); };
    } },
  } as unknown as NonNullable<Window["__TAURI__"]>;
  function View({ terminal }: { terminal?: TerminalSessionId }) { return <span>{useServerStatus(terminal)?.cpuPercent}</span>; }
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<View terminal={first} />));
    expect(invoke).toHaveBeenLastCalledWith("select_server_status_connection", { connectionId: status.connectionId });
    act(() => emit(status));
    expect(container.textContent).toBe("25.5");
    await act(async () => root.render(<View terminal={replacement} />));
    act(() => emit(status));
    expect(container.textContent).toBe("");
    act(() => emit(next));
    expect(container.textContent).toBe("77");
    await act(async () => root.render(<View />));
    expect(invoke).toHaveBeenLastCalledWith("select_server_status_connection", { connectionId: null });
    expect(listeners.size).toBe(0);
    expect(container.textContent).toBe("");
  } finally {
    act(() => root.unmount());
    delete window.__TAURI__;
  }
});
