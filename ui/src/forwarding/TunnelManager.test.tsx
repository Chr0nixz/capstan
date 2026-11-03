import { act, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M3-contract.json";
import { copy } from "../i18n";
import type { HostView } from "../terminal/contracts";
import { decodeTunnelConfig, type TunnelConfig, type TunnelRuntimeSnapshot } from "./contracts";
import { editDraft, draftConfig, tunnelRoute, TunnelManager } from "./TunnelManager";
import { useTunnelTargets } from "./targets";

Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });

const hostId = fixture.tunnels[0].host_id;
const connectionId = "12345678-1234-4234-8234-123456789abc";
const host: HostView = {
  id: hostId as HostView["id"], name: "Ops host", address: "ops.example.test", port: 22,
  username: "operator", credential_id: "23456789-1234-4234-8234-123456789abc" as HostView["credential_id"],
  group_id: null, tags: [], favorite: false, revision: 0, working_directory: null, startup_command: null,
  encoding: "utf-8", theme: "graphite", agent_forwarding: false, production: false,
};
const tunnel = decodeTunnelConfig(fixture.tunnels[0]);
const stopped: TunnelRuntimeSnapshot = {
  id: tunnel.id, state: "stopped", error_code: null, active_connections: 0, bytes_sent: "0" as TunnelRuntimeSnapshot["bytes_sent"], bytes_received: "0" as TunnelRuntimeSnapshot["bytes_received"],
};
const online = { connectionId, generation: "7", state: "online", errorCode: null };

let container: HTMLDivElement;
let root: Root;
let calls: Array<{ command: string; args?: Record<string, unknown> }>;
let invocation: ReturnType<typeof vi.fn>;
let subscriptions: Map<string, (event: { payload: unknown }) => void>;
let configList: TunnelConfig[];
let snapshot: TunnelRuntimeSnapshot;

function hostWindow(): NonNullable<Window["__TAURI__"]> {
  const runtime = {
    core: {
      Channel: class { onmessage: ((message: unknown) => void) | null = null; },
      invoke: (command: string, args?: Record<string, unknown>): Promise<unknown> => invocation(command, args),
    },
    event: {
      listen: async (name: string, callback: (event: { payload: unknown }) => void): Promise<() => void> => {
        subscriptions.set(name, callback);
        return () => { subscriptions.delete(name); };
      },
    },
  };
  return runtime as unknown as NonNullable<Window["__TAURI__"]>;
}

beforeEach(() => {
  calls = [];
  subscriptions = new Map();
  configList = [tunnel];
  snapshot = stopped;
  useTunnelTargets.setState({ hosts: { [hostId]: connectionId } });
  invocation = vi.fn(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    calls.push({ command, args });
    switch (command) {
      case "list_tunnels": return configList;
      case "tunnel_runtime_snapshots": return [snapshot];
      case "connection_snapshots": return [online];
      case "start_tunnel": snapshot = { ...stopped, state: "running" }; return snapshot;
      case "stop_tunnel": snapshot = stopped; return snapshot;
      case "save_tunnel": return { revision: "2" };
      case "delete_tunnel": return { revision: "2" };
      default: return undefined;
    }
  });
  Object.defineProperty(window, "__TAURI__", { configurable: true, value: hostWindow() });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  delete (window as Window & { __TAURI__?: unknown }).__TAURI__;
});

async function mount(): Promise<void> {
  await act(async () => root.render(<TunnelManager
    labels={copy.en} hosts={[host]} revision="1" confirm={async () => true} onRefreshVault={async () => undefined}
    onConnectHost={() => undefined} onClose={() => undefined} /> as ReactElement));
  await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
}
function button(label: string): HTMLButtonElement {
  const result = [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === label);
  if (!result) throw new Error(`Missing button: ${label}`);
  return result;
}

describe("tunnel panel", () => {
  it("round-trips all forwarding modes and formats IPv6 endpoints", () => {
    for (const value of fixture.tunnels) {
      const config = decodeTunnelConfig(value);
      expect(draftConfig(editDraft(config))).toEqual(config);
    }
    expect(tunnelRoute(decodeTunnelConfig({
      ...fixture.tunnels[0], forward: { ...fixture.tunnels[0].forward, bind_host: "::1" },
    }))).toContain("[::1]");
    const invalid = editDraft(tunnel);
    expect(() => draftConfig({ ...invalid, port: "65536" })).toThrow("forwarding_contract_invalid");
  });

  it("starts the selected host generation, reflects traffic events, and waits for stop", async () => {
    await mount();
    await act(async () => button(copy.en.tunnelStart).click());
    expect(calls.find((call) => call.command === "start_tunnel")?.args).toEqual({ id: tunnel.id, connectionId, expectedGeneration: "7" });
    expect(container.querySelector(".tunnel-state")?.textContent).toBe(copy.en.tunnelRunning);

    const events = subscriptions.get("tunnel-state-changed");
    expect(events).toBeDefined();
    await act(async () => events?.({ payload: {
      ...stopped, state: "running", active_connections: 2,
      bytes_sent: "9007199254740993", bytes_received: "4096",
    } }));
    // Readable units on the row; the exact u64 count survives in the tooltip.
    expect(container.querySelector(".tunnel-traffic")?.textContent).toContain("Sent 8.0 PiB · Received 4.0 KiB");
    expect(container.querySelector(".tunnel-traffic")?.getAttribute("title")).toContain("9,007,199,254,740,993 B");
    expect(container.querySelector(".tunnel-traffic")?.textContent).toContain("2 connections");
    expect(container.querySelector(".tunnel-state .led")?.getAttribute("data-state")).toBe("live");

    await act(async () => button(copy.en.tunnelStop).click());
    expect(calls.some((call) => call.command === "stop_tunnel" && call.args?.id === tunnel.id)).toBe(true);
    expect(container.querySelector(".tunnel-state")?.textContent).toBe(copy.en.tunnelStopped);
  });

  it("keeps start disabled until the matching host is online", async () => {
    invocation.mockImplementation(async (command: string): Promise<unknown> => {
      if (command === "list_tunnels") return configList;
      if (command === "tunnel_runtime_snapshots") return [stopped];
      if (command === "connection_snapshots") return [];
      return undefined;
    });
    await mount();
    expect(button(copy.en.tunnelStart).disabled).toBe(true);
    expect(container.textContent).toContain(copy.en.tunnelConnectFirst);
  });

  it("keeps a failed remote cancellation visible and offers a stop retry", async () => {
    snapshot = { ...stopped, state: "failed", error_code: "remote_forward_rejected" };
    await mount();
    expect(container.querySelector(".tunnel-state")?.textContent).toBe(copy.en.tunnelFailed);
    expect(container.textContent).toContain(copy.en.tunnelRemoteRejected);
    await act(async () => button(copy.en.tunnelRetryStop).click());
    expect(calls.some((call) => call.command === "stop_tunnel" && call.args?.id === tunnel.id)).toBe(true);
    expect(container.querySelector(".tunnel-state")?.textContent).toBe(copy.en.tunnelStopped);
  });

  it("allows stopping a pending start and its sibling, and ignores the late start response", async () => {
    const sibling = decodeTunnelConfig({ ...fixture.tunnels[1], id: "b6776ad4-1234-4234-8234-123456789abc", host_id: hostId });
    configList = [tunnel, sibling];
    let finishStart: ((value: unknown) => void) | undefined;
    const originalInvoke = invocation.getMockImplementation();
    invocation.mockImplementation((command: string, args?: Record<string, unknown>): Promise<unknown> => {
      if (command === "tunnel_runtime_snapshots") return Promise.resolve([stopped, { ...stopped, id: sibling.id, state: "running" }]);
      if (command === "start_tunnel") {
        calls.push({ command, args });
        return new Promise((resolve) => { finishStart = resolve; });
      }
      if (command === "stop_tunnel") {
        calls.push({ command, args });
        return Promise.resolve({ ...stopped, id: args?.id });
      }
      return Promise.resolve(originalInvoke?.(command, args));
    });
    await mount();
    await act(async () => button(copy.en.tunnelStart).click());
    await act(async () => subscriptions.get("tunnel-state-changed")?.({ payload: { ...stopped, state: "starting" } }));
    const rows = container.querySelectorAll(".tunnel-list > li");
    const stop = (row: Element): HTMLButtonElement => {
      const result = [...row.querySelectorAll("button")].find((item) => item.textContent === copy.en.tunnelStop);
      if (!result) throw new Error("Missing stop button");
      return result;
    };
    expect(stop(rows[1]).disabled).toBe(false);
    await act(async () => stop(rows[1]).click());
    expect(rows[1].getAttribute("data-state")).toBe("stopped");
    expect(stop(rows[0]).disabled).toBe(false);
    await act(async () => stop(rows[0]).click());
    await act(async () => finishStart?.({ ...stopped, state: "running" }));
    expect(rows[0].getAttribute("data-state")).toBe("stopped");
    expect(calls.filter((call) => call.command === "stop_tunnel").map((call) => call.args?.id)).toEqual([sibling.id, tunnel.id]);
  });
});
