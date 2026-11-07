import { afterEach, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M4-contract.json";
import { createTauriTransport } from "../terminal/transport";
import { useWorkspace } from "../workspace";
import { paneLeaves } from "../panes/tree";

const initialWorkspace = useWorkspace.getState();
afterEach(() => {
  useWorkspace.setState(initialWorkspace, true);
  Reflect.deleteProperty(window, "__TAURI__");
});

it("keeps only the nested opaque request ID through transport, splits and restarts", async () => {
  const invoke = vi.fn(async (_command: string, _args?: Record<string, unknown>) => ({ kind: "local", id: "local-recipe" }));
  Object.defineProperty(window, "__TAURI__", { configurable: true, value: {
    core: { invoke, Channel: class { onmessage: ((value: unknown) => void) | null = null; } },
  } });
  useWorkspace.getState().addTerminal(fixture.localLaunchOptions);
  const tab = useWorkspace.getState().tabs.find((tab) => tab.id === useWorkspace.getState().activeTab)!;
  const transport = createTauriTransport();
  await transport.start(tab.launch, () => undefined, () => undefined);
  useWorkspace.getState().splitPane(tab.id, tab.focusedPaneId, "horizontal");
  let current = useWorkspace.getState().tabs.find((item) => item.id === tab.id)!;
  const added = paneLeaves(current.root).find((pane) => pane.paneId !== tab.focusedPaneId)!;
  await transport.start(added.launch, () => undefined, () => undefined);
  useWorkspace.getState().restartPane(tab.id, tab.focusedPaneId);
  current = useWorkspace.getState().tabs.find((item) => item.id === tab.id)!;
  await transport.start(paneLeaves(current.root).find((pane) => pane.paneId === tab.focusedPaneId)!.launch, () => undefined, () => undefined);
  expect(invoke).toHaveBeenCalledTimes(3);
  for (const [command, args] of invoke.mock.calls) {
    expect(command).toBe("start_local_terminal");
    expect(args?.options).toEqual(fixture.localLaunchOptions);
    expect(Object.keys(args?.options as object)).toEqual(["launch_request_id"]);
    expect(args).not.toHaveProperty("launchRequestId");
  }
});
