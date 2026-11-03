import { listenTauriEvent } from "../terminal/bridge";
import { decodeTunnelConfig, decodeTunnelRuntimeSnapshot, tunnelStateChangedEvent, type TunnelConfig, type TunnelRuntimeSnapshot } from "./contracts";
import type { ConnectionSnapshot } from "../connections/contracts";

type TunnelCommand = "list_tunnels" | "tunnel_runtime_snapshots" | "save_tunnel" | "delete_tunnel" | "start_tunnel" | "stop_tunnel";
async function invoke(command: TunnelCommand, args?: Record<string, unknown>): Promise<unknown> {
  if (!window.__TAURI__) throw new Error("runtime_unavailable");
  return window.__TAURI__.core.invoke<unknown>(command, args);
}
function array(value: unknown): unknown[] {
  if (!Array.isArray(value)) throw new Error("forwarding_contract_invalid");
  return value;
}
export const tunnelApi = {
  list: async () => array(await invoke("list_tunnels")).map(decodeTunnelConfig),
  snapshots: async () => array(await invoke("tunnel_runtime_snapshots")).map(decodeTunnelRuntimeSnapshot),
  save: (config: TunnelConfig, expectedRevision: string) => invoke("save_tunnel", { config, expectedRevision }),
  delete: (id: string, expectedRevision: string) => invoke("delete_tunnel", { id, expectedRevision }),
  start: async (id: string, connection: ConnectionSnapshot) => decodeTunnelRuntimeSnapshot(await invoke("start_tunnel", {
    id, connectionId: connection.connectionId, expectedGeneration: connection.generation,
  })),
  stop: async (id: string) => decodeTunnelRuntimeSnapshot(await invoke("stop_tunnel", { id })),
};

/** Events observed while the initial query is in flight take precedence. */
export async function watchTunnels(receive: (snapshot: TunnelRuntimeSnapshot) => void): Promise<() => void> {
  let disposed = false;
  const observed = new Set<string>();
  const unlisten = await listenTauriEvent<unknown>(tunnelStateChangedEvent, (value) => {
    if (disposed) return;
    try {
      const snapshot = decodeTunnelRuntimeSnapshot(value);
      observed.add(snapshot.id);
      receive(snapshot);
    } catch { /* Malformed events cannot update runtime state. */ }
  });
  try {
    for (const snapshot of await tunnelApi.snapshots()) if (!disposed && !observed.has(snapshot.id)) receive(snapshot);
  } catch (error) { disposed = true; unlisten(); throw error; }
  return () => { disposed = true; unlisten(); };
}
