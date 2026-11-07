import { useEffect, useRef, useState } from "react";
import { isTauriAvailable, listenTauriEvent } from "../terminal/bridge";
import type { TerminalSessionId } from "../terminal/contracts";
import { decodeServerStatus, decodeStatusBinding, matchesStatus, serverStatusEvent, type ServerStatusSnapshot, type StatusBinding } from "./contracts";

function invoke(command: string, args: Record<string, unknown>): Promise<unknown> {
  return window.__TAURI__?.core.invoke<unknown>(command, args) ?? Promise.reject(new Error("status_closed"));
}

export function useServerStatus(terminal: TerminalSessionId | undefined): ServerStatusSnapshot | undefined {
  const [current, setCurrent] = useState<{ terminal: TerminalSessionId; snapshot: ServerStatusSnapshot }>();
  const pendingSelection = useRef(Promise.resolve());
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const select = (connectionId: StatusBinding["connectionId"] | null) => {
      // An earlier pane's in-flight IPC must finish before its replacement can select.
      pendingSelection.current = pendingSelection.current.then(async () => {
        if (!disposed) await invoke("select_server_status_connection", { connectionId });
      }).catch(() => undefined);
      return pendingSelection.current;
    };
    // Drop the previous selection while looking up this pane's logical connection.
    void select(null);
    if (!terminal) return;
    void (async () => {
      const value = await invoke("server_status_binding", { terminal });
      if (disposed || value === null) return;
      const binding = decodeStatusBinding(value);
      const stop = await listenTauriEvent<unknown>(serverStatusEvent, (value) => {
        if (disposed) return;
        if (value === null) { setCurrent(undefined); return; }
        try {
          const snapshot = decodeServerStatus(value);
          if (matchesStatus(binding, snapshot)) setCurrent({ terminal, snapshot });
        } catch { /* Ignore malformed status events. */ }
      });
      if (disposed) { stop(); return; }
      unlisten = stop;
      await select(binding.connectionId);
    })().catch(() => { if (!disposed) setCurrent(undefined); });
    return () => { disposed = true; unlisten?.(); };
  }, [terminal]);
  return current && current.terminal === terminal ? current.snapshot : undefined;
}

export function retryServerStatus({ connectionId, generation }: StatusBinding): Promise<unknown> {
  return invoke("retry_server_status", { connectionId, generation });
}
