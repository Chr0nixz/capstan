import { createStore } from "zustand/vanilla";
import { invokeTauri } from "../terminal/bridge";
import { decodeExactU64, type ExactU64 } from "./contracts";

export type PanelLayout = { dock: "right" | "bottom"; size: number; collapsed: boolean };
type Snapshot = { layout: PanelLayout; session: string; sequence: ExactU64 };
type Transport = { load(): Promise<Snapshot>; save(snapshot: Snapshot): Promise<void> };
type State = { layout: PanelLayout; tabs: Record<string, PanelLayout>; loading: boolean; saving: boolean; error: "load" | "save" | null };
const defaultLayout: PanelLayout = { dock: "right", size: 36, collapsed: false };

export function decodePanelLayout(value: unknown): PanelLayout {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("settings_invalid");
  const data = value as Record<string, unknown>;
  if (Object.keys(data).length !== 3 || (data.dock !== "right" && data.dock !== "bottom")
    || typeof data.size !== "number" || !Number.isInteger(data.size) || data.size < 20 || data.size > 60 || typeof data.collapsed !== "boolean") {
    throw new Error("settings_invalid");
  }
  return { dock: data.dock, size: data.size, collapsed: data.collapsed };
}

function decodeSnapshot(value: unknown): Snapshot {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("settings_invalid");
  const data = value as Record<string, unknown>;
  if (Object.keys(data).length !== 3 || typeof data.session !== "string"
    || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(data.session)) throw new Error("settings_invalid");
  return { layout: decodePanelLayout(data.layout), session: data.session, sequence: decodeExactU64(data.sequence) };
}

export function createPanelPreferences(transport: Transport) {
  const store = createStore<State>(() => ({ layout: defaultLayout, tabs: {}, loading: false, saving: false, error: null }));
  let initialization: Promise<void> | undefined;
  let session: string | null = null, sequence = 0n, revision = 0n;
  let dirty = false;
  let hydrated = false, epoch = 0;
  let knownIds: string[] = [];
  const rememberTabs = (): void => {
    const state = store.getState();
    const tabs = Object.fromEntries(knownIds.filter((id) => state.tabs[id] || hydrated).map((id) => [id, state.tabs[id] ?? state.layout]));
    if (Object.keys(tabs).length !== Object.keys(state.tabs).length || Object.keys(tabs).some((id) => !state.tabs[id])) store.setState({ tabs });
  };
  const save = (): void => {
    if (!session) return;
    if (sequence === 18446744073709551615n) { store.setState({ error: "save", saving: false }); return; }
    const operation = revision, generation = epoch;
    const snapshot = { layout: store.getState().layout, session, sequence: (++sequence).toString() as ExactU64 };
    store.setState({ saving: true });
    void transport.save(snapshot).then(() => {
      if (revision === operation && generation === epoch) { dirty = false; store.setState({ saving: false, error: null }); }
    }).catch(() => {
      if (revision === operation && generation === epoch) store.setState({ saving: false, error: "save" });
    });
  };
  const initialize = (): Promise<void> => {
    initialization ??= (async () => {
      epoch++;
      store.setState({ loading: true });
      try {
        const snapshot = await transport.load();
        session = snapshot.session; sequence = BigInt(snapshot.sequence);
        if (!dirty) store.setState({ layout: snapshot.layout });
        store.setState({ loading: false, error: null });
        hydrated = true; rememberTabs();
        if (dirty) save();
      } catch { hydrated = true; store.setState({ loading: false, error: "load" }); rememberTabs(); }
    })();
    return initialization;
  };
  return {
    store, initialize,
    set(tab: string, value: PanelLayout): void {
      const layout = decodePanelLayout(value);
      revision++; dirty = true;
      hydrated = true; rememberTabs();
      store.setState((state) => ({ layout, tabs: { ...state.tabs, [tab]: layout } }));
      save();
    },
    prune(ids: string[]): void {
      knownIds = ids; rememberTabs();
    },
    retry(): Promise<void> {
      if (store.getState().loading) return initialization ?? Promise.resolve();
      initialization = undefined;
      return initialize();
    },
  };
}

export const panelPreferences = createPanelPreferences({
  load: async () => decodeSnapshot(await invokeTauri("load_sftp_panel_preferences")),
  save: (snapshot) => invokeTauri("save_sftp_panel_preferences", snapshot),
});
