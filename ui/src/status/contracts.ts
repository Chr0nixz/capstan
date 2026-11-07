import type { ConnectionId } from "../connections/contracts";
import { decodeExactU64, type ExactU64 } from "../sftp/contracts";

export const serverStatusEvent = "server-status-changed";
export const statusStates = ["probing", "running", "paused", "unsupported", "unavailable"] as const;
export const statusErrors = ["status_unsupported", "status_rejected", "status_parse", "status_limit", "status_closed"] as const;
export const statusDetailLimits = { interfaces: 64, mounts: 64 } as const;
type StatusState = typeof statusStates[number];
type StatusError = typeof statusErrors[number];
export type StatusBinding = { connectionId: ConnectionId; generation: ExactU64 };
type Memory = { total: ExactU64; available: ExactU64 };
type Disk = { mount: string; total: ExactU64; used: ExactU64 };
type Rate = { rxPerSec: ExactU64; txPerSec: ExactU64 };
type Details = { disks: Disk[]; interfaces: { name: string; rate?: Rate }[] };
export type ServerStatusSnapshot = StatusBinding & {
  state: StatusState; errorCode?: StatusError; sampledAt: ExactU64; cpuPercent?: number;
  load?: [number, number, number]; memory?: Memory; disk?: Disk; network?: Rate; details?: Details;
};

const invalid = (): never => { throw new Error("server_status_contract_invalid"); };
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  return value as Record<string, unknown>;
}
function label(value: unknown): string {
  return typeof value === "string" && value.length > 0 && value.length <= 65536 && !/\p{Cc}/u.test(value) ? value : invalid();
}
function metric(value: unknown, max = Infinity): number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= max ? value : invalid();
}
function optional<T>(value: unknown, decode: (value: unknown) => T): T | undefined {
  return value === undefined ? undefined : decode(value);
}
function choice<T extends string>(value: unknown, choices: readonly T[]): T {
  return choices.find((choice) => choice === value) ?? invalid();
}

function disk(value: unknown): Disk {
  const data = record(value);
  return { mount: label(data.mount), total: decodeExactU64(data.total), used: decodeExactU64(data.used) };
}
function memory(value: unknown): Memory {
  const data = record(value);
  const total = decodeExactU64(data.total), available = decodeExactU64(data.available);
  if (BigInt(total) === 0n || BigInt(available) > BigInt(total)) return invalid();
  return { total, available };
}
function rate(value: unknown): Rate {
  const data = record(value);
  return { rxPerSec: decodeExactU64(data.rxPerSec), txPerSec: decodeExactU64(data.txPerSec) };
}
function details(value: unknown): Details {
  const data = record(value);
  if (!Array.isArray(data.disks) || data.disks.length > statusDetailLimits.mounts
    || !Array.isArray(data.interfaces) || data.interfaces.length > statusDetailLimits.interfaces) return invalid();
  return { disks: data.disks.map(disk), interfaces: data.interfaces.map((value: unknown) => {
    const item = record(value);
    return { name: label(item.name), rate: optional(item.rate, rate) };
  }) };
}

export function decodeStatusBinding(value: unknown): StatusBinding {
  const data = record(value);
  const id = data.connectionId, generation = decodeExactU64(data.generation);
  if (typeof id !== "string" || !/^[\da-f]{8}(?:-[\da-f]{4}){3}-[\da-f]{12}$/iu.test(id)
    || id === "00000000-0000-0000-0000-000000000000" || String(generation) === "0") return invalid();
  return { connectionId: id as ConnectionId, generation };
}

export function decodeServerStatus(value: unknown): ServerStatusSnapshot {
  const data = record(value);
  const load = optional(data.load, (value): [number, number, number] => {
    if (!Array.isArray(value) || value.length !== 3) return invalid();
    return [metric(value[0]), metric(value[1]), metric(value[2])];
  });
  const root = optional(data.disk, disk);
  if (root && root.mount !== "/") return invalid();
  return {
    ...decodeStatusBinding(value), state: choice(data.state, statusStates), errorCode: optional(data.errorCode, (code) => choice(code, statusErrors)),
    sampledAt: decodeExactU64(data.sampledAt), cpuPercent: optional(data.cpuPercent, (cpu) => metric(cpu, 100)), load,
    memory: optional(data.memory, memory), disk: root, network: optional(data.network, rate), details: optional(data.details, details),
  };
}

export function matchesStatus(binding: StatusBinding, snapshot: ServerStatusSnapshot): boolean {
  return snapshot.connectionId === binding.connectionId && snapshot.generation === binding.generation;
}
