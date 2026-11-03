import { useEffect, useRef, useState, type FormEvent, type KeyboardEventHandler, type ReactElement, type Ref } from "react";
import { copy } from "../i18n";
import { CloseIcon, PencilIcon, PlusIcon, TrashIcon, TunnelIcon } from "../ui/icons";
import { formatBytes } from "../ui/format";
import type { ConfirmRequest } from "../ui/ConfirmDialog";
import type { HostView } from "../terminal/contracts";
import { watchConnections } from "../connections/feed";
import type { ConnectionSnapshot } from "../connections/contracts";
import { decodeTunnelConfig, defaultLocalBindHost, type TunnelConfig, type TunnelForward, type TunnelRuntimeSnapshot } from "./contracts";
import { tunnelApi, watchTunnels } from "./api";
import { useTunnelTargets } from "./targets";
import { tunnelError } from "./errors";

type Labels = typeof copy.en | typeof copy["zh-CN"];
type Draft = { id: string; host: string; kind: TunnelForward["kind"]; bind: string; port: string; destination: string; destinationPort: string; auto: boolean };
type Props = {
  labels: Labels; hosts: readonly HostView[]; revision: string;
  confirm: (request: ConfirmRequest) => Promise<boolean>;
  onRefreshVault: () => Promise<void>; onConnectHost: (host: HostView) => void; onClose: () => void;
  panelRef?: Ref<HTMLElement>; onKeyDown?: KeyboardEventHandler<HTMLElement>;
};
// Tunnel states share the session signal lights: quiet when stopped, green when running.
const tunnelSignal: Record<TunnelRuntimeSnapshot["state"], "ended" | "connecting" | "live" | "failed"> = {
  stopped: "ended", starting: "connecting", running: "live", stopping: "connecting", failed: "failed",
};
const freshDraft = (host: string): Draft => ({ id: crypto.randomUUID(), host, kind: "local", bind: defaultLocalBindHost, port: "", destination: defaultLocalBindHost, destinationPort: "", auto: false });
export function editDraft(config: TunnelConfig): Draft {
  const forward = config.forward;
  return { id: config.id, host: config.host_id, kind: forward.kind, auto: config.auto_start,
    bind: forward.kind === "remote" ? forward.remote_bind_host : forward.bind_host,
    port: String(forward.kind === "remote" ? forward.remote_bind_port : forward.bind_port),
    destination: forward.kind === "local" ? forward.destination_host : forward.kind === "remote" ? forward.local_destination_host : defaultLocalBindHost,
    destinationPort: forward.kind === "local" ? String(forward.destination_port) : forward.kind === "remote" ? String(forward.local_destination_port) : "",
  };
}
export function draftConfig(draft: Draft): TunnelConfig {
  const forward = draft.kind === "local" ? { kind: draft.kind, bind_host: draft.bind, bind_port: Number(draft.port), destination_host: draft.destination, destination_port: Number(draft.destinationPort) }
    : draft.kind === "remote" ? { kind: draft.kind, remote_bind_host: draft.bind, remote_bind_port: Number(draft.port), local_destination_host: draft.destination, local_destination_port: Number(draft.destinationPort) }
      : { kind: draft.kind, bind_host: draft.bind, bind_port: Number(draft.port) };
  return decodeTunnelConfig({ id: draft.id, host_id: draft.host, auto_start: draft.auto, forward });
}
function endpoint(host: string, port: number): string { return `${host.includes(":") ? `[${host}]` : host}:${port}`; }
export function tunnelRoute(config: TunnelConfig): string {
  const forward = config.forward;
  return forward.kind === "local" ? `${endpoint(forward.bind_host, forward.bind_port)} → ${endpoint(forward.destination_host, forward.destination_port)}`
    : forward.kind === "remote" ? `${endpoint(forward.remote_bind_host, forward.remote_bind_port)} → ${endpoint(forward.local_destination_host, forward.local_destination_port)}`
      : endpoint(forward.bind_host, forward.bind_port);
}

export function TunnelManager({ labels, hosts, revision, confirm, onRefreshVault, onConnectHost, onClose, panelRef, onKeyDown }: Props): ReactElement {
  const targets = useTunnelTargets((state) => state.hosts);
  const [configs, setConfigs] = useState<TunnelConfig[]>([]);
  const [snapshots, setSnapshots] = useState<Record<string, TunnelRuntimeSnapshot>>({});
  const [connections, setConnections] = useState<Record<string, ConnectionSnapshot>>({});
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState("");
  const [pendingKeys, setPendingKeys] = useState<ReadonlySet<string>>(new Set());
  const [loading, setLoading] = useState(true);
  const pending = useRef(new Set<string>());
  const runtimeRequests = useRef(new Map<string, number>());
  const busy = pendingKeys.has("config");
  const alive = useRef(true);
  const editorRef = useRef<HTMLFormElement>(null);
  useEffect(() => { if (draft) editorRef.current?.querySelector<HTMLElement>("select")?.focus(); }, [draft?.id]);
  useEffect(() => {
    alive.current = true;
    let disposed = false;
    const cleanup: Array<() => void> = [];
    const install = async (watch: Promise<() => void>): Promise<void> => {
      const stop = await watch;
      if (disposed) stop(); else cleanup.push(stop);
    };
    void Promise.all([
      tunnelApi.list().then((next) => { if (!disposed) setConfigs(next); }),
      install(watchTunnels((snapshot) => { if (!disposed) setSnapshots((current) => ({ ...current, [snapshot.id]: snapshot })); })),
      install(watchConnections((snapshot) => { if (!disposed) setConnections((current) => ({ ...current, [snapshot.connectionId]: snapshot })); })),
    ]).catch(() => { if (!disposed) setError(labels.tunnelLoadFailed); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; alive.current = false; cleanup.forEach((stop) => stop()); };
  }, []);
  const run = async (operation: () => Promise<void>, key = "config"): Promise<void> => {
    if (pending.current.has(key)) return;
    pending.current.add(key); setPendingKeys(new Set(pending.current)); setError("");
    try { await operation(); }
    catch (cause) {
      await onRefreshVault().catch(() => undefined);
      try { const next = await tunnelApi.list(); if (alive.current) setConfigs(next); } catch { /* Keep the current draft on errors. */ }
      if (alive.current) setError(tunnelError(cause, labels));
    } finally { pending.current.delete(key); if (alive.current) setPendingKeys(new Set(pending.current)); }
  };
  const changeRuntime = (id: string, action: "start" | "stop", operation: () => Promise<TunnelRuntimeSnapshot>): void => {
    const key = `${action}:${id}`;
    if (pending.current.has(key)) return;
    const request = (runtimeRequests.current.get(id) ?? 0) + 1;
    runtimeRequests.current.set(id, request);
    void run(async () => {
      const next = await operation();
      // A delayed start response cannot undo a later stop command.
      if (alive.current && runtimeRequests.current.get(id) === request) setSnapshots((current) => ({ ...current, [next.id]: next }));
    }, key);
  };
  const refresh = async (): Promise<void> => {
    await onRefreshVault();
    const next = await tunnelApi.list();
    if (alive.current) setConfigs(next);
  };
  const save = (event: FormEvent): void => {
    event.preventDefault();
    if (!draft || pending.current.has("config")) return;
    let config: TunnelConfig;
    try { config = draftConfig(draft); } catch { setError(labels.tunnelInvalid); return; }
    void run(async () => {
      await tunnelApi.save(config, revision);
      await refresh();
      setSnapshots((current) => { const next = { ...current }; delete next[config.id]; return next; });
      setDraft(null);
    });
  };
  const kindLabel = (kind: TunnelForward["kind"]): string => kind === "local" ? labels.tunnelLocal : kind === "remote" ? labels.tunnelRemote : labels.tunnelSocks;
  const stateLabel = (state: TunnelRuntimeSnapshot["state"]): string => ({ stopped: labels.tunnelStopped, starting: labels.tunnelStarting, running: labels.tunnelRunning, stopping: labels.tunnelStopping, failed: labels.tunnelFailed })[state];
  return <aside ref={panelRef} className="sheet tunnel-manager" role="dialog" aria-labelledby="tunnel-title" tabIndex={-1} onKeyDown={onKeyDown}>
    <header className="sheet-header"><h2 id="tunnel-title"><TunnelIcon size={14} />{labels.tunnels}</h2><button className="icon-button" type="button" title={labels.closePanel} aria-label={labels.closePanel} onClick={onClose}><CloseIcon /></button></header>
    <div className="sheet-body">
      <div className="tunnel-toolbar"><button type="button" disabled={busy || loading || hosts.length === 0} onClick={() => { setError(""); setDraft(freshDraft(hosts[0]?.id ?? "")); }}><PlusIcon size={14} />{labels.tunnelNew}</button></div>
      {error && <p role="alert" className="inline-error">{error}</p>}
      {loading ? <p role="status">{labels.tunnelLoading}</p> : configs.length === 0 && <p className="tunnel-help">{hosts.length === 0 ? labels.noHosts : labels.tunnelEmpty}</p>}
      <ul className="tunnel-list" aria-label={labels.tunnels}>{configs.map((config) => {
        const host = hosts.find((item) => item.id === config.host_id);
        const snapshot = snapshots[config.id];
        const state = snapshot?.state ?? "stopped";
        const connection = connections[targets[config.host_id] ?? ""];
        const online = connection?.state === "online";
        const running = state === "running" || state === "starting" || state === "stopping";
        const retryStop = state === "failed" && snapshot?.error_code === "remote_forward_rejected";
        return <li key={config.id} data-state={state}>
          <div className="tunnel-row-heading"><strong>{host?.name ?? labels.tunnelHost}</strong>{host?.production && <span className="prod-chip">{labels.production}</span>}
            <span className="tunnel-state"><span className="led" data-state={tunnelSignal[state]} aria-hidden="true" />{stateLabel(state)}</span></div>
          <span className="tunnel-help">{kindLabel(config.forward.kind)}{config.auto_start ? ` · ${labels.tunnelAuto}` : ""}</span>
          <code className="tunnel-route">{tunnelRoute(config)}</code>
          {/* Exact byte counts stay in the tooltip; the row reads at a glance. */}
          <p className="tunnel-traffic" title={labels.tunnelTraffic(snapshot?.active_connections ?? 0, `${BigInt(snapshot?.bytes_sent ?? "0").toLocaleString()} B`, `${BigInt(snapshot?.bytes_received ?? "0").toLocaleString()} B`)}>
            {labels.tunnelTraffic(snapshot?.active_connections ?? 0, formatBytes(snapshot?.bytes_sent ?? "0"), formatBytes(snapshot?.bytes_received ?? "0"))}</p>
          {snapshot?.error_code && <p className="field-error" role="status">{tunnelError(snapshot.error_code, labels)}</p>}
          {!running && !online && <p className="tunnel-help">{labels.tunnelConnectFirst}</p>}
          <div className="tunnel-actions">
            {running || retryStop ? <button type="button" disabled={pendingKeys.has(`stop:${config.id}`) || state === "stopping"} onClick={() => changeRuntime(config.id, "stop", () => tunnelApi.stop(config.id))}>{retryStop ? labels.tunnelRetryStop : labels.tunnelStop}</button>
              : <button type="button" disabled={busy || !online || pendingKeys.has(`start:${config.id}`)} onClick={() => { if (connection) changeRuntime(config.id, "start", () => tunnelApi.start(config.id, connection)); }}>{labels.tunnelStart}</button>}
            {!online && host && <button type="button" disabled={busy} onClick={() => onConnectHost(host)}>{labels.connectHost}</button>}
            <button className="icon-button" type="button" disabled={busy} title={labels.tunnelEdit} aria-label={`${labels.tunnelEdit}: ${tunnelRoute(config)}`} onClick={() => { setDraft(editDraft(config)); setError(""); }}><PencilIcon size={14} /></button>
            <button className="icon-button" type="button" disabled={busy} title={labels.tunnelDelete} aria-label={`${labels.tunnelDelete}: ${tunnelRoute(config)}`} onClick={() => {
              void confirm({
                title: labels.tunnelDeleteTitle(tunnelRoute(config)), body: labels.tunnelDeleteConfirm,
                confirm: labels.tunnelDelete, cancel: labels.cancelEdit, tone: "danger",
              }).then((accepted) => {
                if (accepted) void run(async () => { await tunnelApi.delete(config.id, revision); await refresh(); });
              });
            }}><TrashIcon size={14} /></button>
          </div>
        </li>;
      })}</ul>
      {draft && <form ref={editorRef} className="tunnel-editor" onSubmit={save} noValidate aria-label={labels.tunnelEdit}>
        <h3>{configs.some((config) => config.id === draft.id) ? labels.tunnelEdit : labels.tunnelNew}</h3>
        <fieldset disabled={busy}>
          <label>{labels.tunnelHost}<select value={draft.host} onChange={(event) => setDraft({ ...draft, host: event.currentTarget.value })}>{hosts.map((host) => <option key={host.id} value={host.id}>{host.name}{host.production ? ` (${labels.production})` : ""}</option>)}</select></label>
          <label>{labels.tunnelKind}<select value={draft.kind} onChange={(event) => setDraft({ ...draft, kind: event.currentTarget.value as Draft["kind"] })}>{(["local", "remote", "socks5"] as const).map((kind) => <option key={kind} value={kind}>{kindLabel(kind)}</option>)}</select></label>
          <p className="tunnel-help">{draft.kind === "local" ? labels.tunnelLocalHint : draft.kind === "remote" ? labels.tunnelRemoteHint : labels.tunnelSocksHint}</p>
          <div className="tunnel-endpoint"><label>{labels.tunnelBindHost}<input value={draft.bind} onChange={(event) => setDraft({ ...draft, bind: event.currentTarget.value })} autoComplete="off" spellCheck={false} /></label><label>{labels.tunnelBindPort}<input type="number" min="1" max="65535" value={draft.port} onChange={(event) => setDraft({ ...draft, port: event.currentTarget.value })} /></label></div>
          <p className="tunnel-help">{labels.tunnelBindHint}</p>
          {draft.kind !== "socks5" && <div className="tunnel-endpoint"><label>{labels.tunnelDestinationHost}<input value={draft.destination} onChange={(event) => setDraft({ ...draft, destination: event.currentTarget.value })} autoComplete="off" spellCheck={false} /></label><label>{labels.tunnelDestinationPort}<input type="number" min="1" max="65535" value={draft.destinationPort} onChange={(event) => setDraft({ ...draft, destinationPort: event.currentTarget.value })} /></label></div>}
          <label className="check-label"><input type="checkbox" checked={draft.auto} onChange={(event) => setDraft({ ...draft, auto: event.currentTarget.checked })} />{labels.tunnelAuto}</label>
        </fieldset>
          <div className="tunnel-actions"><button className="primary-action" type="submit" disabled={busy}>{labels.tunnelSave}</button><button className="ghost-button" type="button" disabled={busy} onClick={() => setDraft(null)}>{labels.cancelEdit}</button></div>
      </form>}
    </div>
  </aside>;
}
