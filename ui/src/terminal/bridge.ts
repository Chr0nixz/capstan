import type { Commands } from "./contracts";

export type TauriChannel<T> = { onmessage: ((message: T) => void) | null };

type TauriGlobal = {
  event?: { listen: <T>(name: string, callback: (event: { payload: T }) => void) => Promise<() => void> };
  core: {
    Channel: new <T>() => TauriChannel<T>;
    // A method, so test doubles may accept only the argument shapes they use.
    invoke<T>(command: string, args?: Record<string, unknown> | Uint8Array, options?: { headers?: Record<string, string> }): Promise<T>;
  };
};

declare global {
  interface Window {
    __TAURI__?: TauriGlobal;
  }
}

export function listenTauriEvent<T>(name: string, callback: (payload: T) => void): Promise<() => void> {
  const events = window.__TAURI__?.event;
  return events ? events.listen<T>(name, (event) => callback(event.payload)) : Promise.reject(new Error("window_events_unavailable"));
}

export function listenSnapEvent(name: "tauri-snap://snap/mouseenter" | "tauri-snap://snap/mouseleave", callback: () => void): Promise<() => void> {
  return listenTauriEvent(name, callback);
}

export function isTauriAvailable(): boolean {
  return window.__TAURI__ !== undefined;
}

export function invokeTauri<K extends keyof Commands>(
  command: K,
  ...args: Commands[K]["args"] extends undefined ? [args?: undefined] : [args: Commands[K]["args"]]
): Promise<Commands[K]["result"]> {
  const tauri = window.__TAURI__;
  if (!tauri) {
    return Promise.reject(new Error("Tauri runtime unavailable"));
  }
  return tauri.core.invoke<Commands[K]["result"]>(command, args[0]);
}

/** Sends `bytes` as the raw request body; `headers` carry the other arguments. */
export function invokeTauriBytes(command: "write_terminal_input", bytes: Uint8Array, headers: Record<string, string>): Promise<void> {
  const tauri = window.__TAURI__;
  if (!tauri) {
    return Promise.reject(new Error("Tauri runtime unavailable"));
  }
  return tauri.core.invoke<void>(command, bytes, { headers });
}

/** Terminal input goes as bytes: a legacy mouse report is not UTF-8. */
export function writeTerminalInput(id: string, bytes: Uint8Array): Promise<void> {
  return invokeTauriBytes("write_terminal_input", bytes, { "capstan-terminal": id });
}

export function createTauriChannel<T>(): TauriChannel<T> | undefined {
  const tauri = window.__TAURI__;
  return tauri ? new tauri.core.Channel<T>() : undefined;
}
