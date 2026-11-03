import type { HostId } from "../terminal/contracts";

export type TunnelId = string & { readonly tunnel: unique symbol };
export type ExactBytes = string & { readonly exactBytes: unique symbol };
export const defaultLocalBindHost = "127.0.0.1" as const;
export const tunnelStateChangedEvent = "tunnel-state-changed" as const;
export type TunnelForward =
  | { kind: "local"; bind_host: string; bind_port: number; destination_host: string; destination_port: number }
  | { kind: "remote"; remote_bind_host: string; remote_bind_port: number; local_destination_host: string; local_destination_port: number }
  | { kind: "socks5"; bind_host: string; bind_port: number };
export type TunnelConfig = { id: TunnelId; host_id: HostId; auto_start: boolean; forward: TunnelForward };
export type TunnelRuntimeState = "stopped" | "starting" | "running" | "stopping" | "failed";
export const tunnelErrorCodes = [
  "bind_address_in_use", "remote_forward_rejected", "destination_unreachable", "connection_lost", "canceled",
  "invalid_config", "ssh_channel_rejected", "ssh_protocol", "client_limit",
] as const;
export type TunnelErrorCode = typeof tunnelErrorCodes[number];
export type TunnelRuntimeSnapshot = {
  id: TunnelId; state: TunnelRuntimeState; error_code: TunnelErrorCode | null;
  active_connections: number; bytes_sent: ExactBytes; bytes_received: ExactBytes;
};

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const nilId = "00000000-0000-0000-0000-000000000000";
const maximumU64 = "18446744073709551615";
const invalid = (): never => { throw new Error("forwarding_contract_invalid"); };

function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalid();
  const result = value as Record<string, unknown>;
  if (Object.keys(result).length !== keys.length || !keys.every((key) => Object.hasOwn(result, key))) return invalid();
  return result;
}

function validHost(value: unknown): value is string {
  if (typeof value !== "string") return false;
  // Keep acceptance aligned with capstan_vault::normalize_target, including scoped IPv6.
  const host = value.replace(/^[\u0009-\u000d\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+|[\u0009-\u000d\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+$/gu, "");
  if (host.length === 0 || host.length > 253 || /[^\u0021-\u007e]/u.test(host)) return false;
  const ip = host.startsWith("[") && host.endsWith("]") ? host.slice(1, -1) : host;
  const scopeIndex = ip.indexOf("%");
  if (scopeIndex !== -1) {
    const scope = ip.slice(scopeIndex + 1);
    return validIpv6(ip.slice(0, scopeIndex)) && /^[A-Za-z0-9_.-]{1,64}$/u.test(scope)
      && (!/^[0-9]+$/u.test(scope) || Number(scope) <= 0xffffffff);
  }
  if (validIpv6(ip) || validIpv4(ip)) return true;
  return host.replace(/\.+$/u, "").split(".")
    .every((label) => /^[A-Za-z0-9](?:[A-Za-z0-9_-]{0,61}[A-Za-z0-9])?$/u.test(label));
}

function validIpv4(value: string): boolean {
  const parts = value.split(".");
  return parts.length === 4 && parts.every((part) => /^(0|[1-9][0-9]{0,2})$/u.test(part) && Number(part) <= 255);
}

function validIpv6(value: string): boolean {
  let address = value;
  if (address.includes(".")) {
    const lastColon = address.lastIndexOf(":");
    if (lastColon === -1 || !validIpv4(address.slice(lastColon + 1))) return false;
    address = `${address.slice(0, lastColon + 1)}0:0`;
  }
  const halves = address.split("::");
  if (halves.length > 2) return false;
  const groups = halves.flatMap((half) => half === "" ? [] : half.split(":"));
  return groups.every((group) => /^[0-9a-f]{1,4}$/iu.test(group))
    && (halves.length === 2 ? groups.length < 8 : groups.length === 8);
}

function validPort(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 1 && value <= 65535;
}

function exactBytes(value: unknown): value is string {
  return typeof value === "string" && /^(0|[1-9][0-9]*)$/u.test(value)
    && value.length <= maximumU64.length && (value.length < maximumU64.length || value <= maximumU64);
}

export function decodeTunnelConfig(value: unknown): TunnelConfig {
  const config = record(value, ["id", "host_id", "auto_start", "forward"]);
  if (typeof config.id !== "string" || !uuidPattern.test(config.id) || config.id === nilId
    || typeof config.host_id !== "string" || !uuidPattern.test(config.host_id) || config.host_id === nilId
    || typeof config.auto_start !== "boolean") return invalid();
  if (typeof config.forward !== "object" || config.forward === null || Array.isArray(config.forward)) return invalid();
  const candidate = config.forward as Record<string, unknown>;
  const keys = candidate.kind === "local"
    ? ["kind", "bind_host", "bind_port", "destination_host", "destination_port"]
    : candidate.kind === "remote"
      ? ["kind", "remote_bind_host", "remote_bind_port", "local_destination_host", "local_destination_port"]
      : candidate.kind === "socks5"
        ? ["kind", "bind_host", "bind_port"]
        : [];
  const forward = record(config.forward, keys);
  if (keys.length === 0) return invalid();
  if (forward.kind === "local") {
    if (!validHost(forward.bind_host) || !validPort(forward.bind_port) || !validHost(forward.destination_host) || !validPort(forward.destination_port)) return invalid();
  } else if (forward.kind === "remote") {
    if (!validHost(forward.remote_bind_host) || !validPort(forward.remote_bind_port)
      || !validHost(forward.local_destination_host) || !validPort(forward.local_destination_port)) return invalid();
  } else if (!validHost(forward.bind_host) || !validPort(forward.bind_port)) return invalid();
  return { ...config, forward } as TunnelConfig;
}

export function decodeTunnelRuntimeSnapshot(value: unknown): TunnelRuntimeSnapshot {
  const snapshot = record(value, ["id", "state", "error_code", "active_connections", "bytes_sent", "bytes_received"]);
  if (typeof snapshot.id !== "string" || !uuidPattern.test(snapshot.id)
    || typeof snapshot.state !== "string" || !["stopped", "starting", "running", "stopping", "failed"].includes(snapshot.state)
    || (snapshot.error_code !== null && !tunnelErrorCodes.includes(snapshot.error_code as TunnelErrorCode))
    || typeof snapshot.active_connections !== "number" || !Number.isInteger(snapshot.active_connections)
    || snapshot.active_connections < 0 || snapshot.active_connections > 0xffffffff
    || !exactBytes(snapshot.bytes_sent) || !exactBytes(snapshot.bytes_received)) return invalid();
  return snapshot as TunnelRuntimeSnapshot;
}
