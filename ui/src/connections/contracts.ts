export type ConnectionId = string & { readonly connection: unique symbol };
export type ExactGeneration = string & { readonly exactGeneration: unique symbol };
export type ConnectionState = "connecting" | "authenticating" | "online" | "reconnecting" | "disconnected" | "failed";
export const connectionErrorCodes = [
  "ssh_timeout", "ssh_connection", "ssh_authentication", "ssh_protocol", "ssh_channel_rejected",
  "ssh_private_key_invalid", "ssh_agent_unavailable", "ssh_agent_identity_unavailable", "host_key_unknown",
  "host_key_changed", "host_key_rejected", "vault_locked", "connection_lost", "reconnect_exhausted", "canceled", "invalid_input",
] as const;
export type ConnectionErrorCode = typeof connectionErrorCodes[number];
export type ConnectionSnapshot = {
  connectionId: ConnectionId; generation: ExactGeneration; state: ConnectionState; errorCode: ConnectionErrorCode | null;
};
export const connectionStateChangedEvent = "connection-state-changed" as const;

const maximumU64 = "18446744073709551615";
const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const invalid = (): never => { throw new Error("connection_contract_invalid"); };

function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalid();
  const result = value as Record<string, unknown>;
  if (Object.keys(result).length !== keys.length || !keys.every((key) => Object.hasOwn(result, key))) return invalid();
  return result;
}

export function decodeConnectionSnapshot(value: unknown): ConnectionSnapshot {
  const snapshot = record(value, ["connectionId", "generation", "state", "errorCode"]);
  if (typeof snapshot.connectionId !== "string" || !uuidPattern.test(snapshot.connectionId)
    || snapshot.connectionId === "00000000-0000-0000-0000-000000000000"
    || typeof snapshot.generation !== "string" || !/^[1-9][0-9]*$/u.test(snapshot.generation)
    || snapshot.generation.length > maximumU64.length
    || (snapshot.generation.length === maximumU64.length && snapshot.generation > maximumU64)
    || typeof snapshot.state !== "string"
    || !["connecting", "authenticating", "online", "reconnecting", "disconnected", "failed"].includes(snapshot.state)
    || (snapshot.errorCode !== null && !connectionErrorCodes.includes(snapshot.errorCode as ConnectionErrorCode))) return invalid();
  return {
    connectionId: snapshot.connectionId as ConnectionId,
    generation: snapshot.generation as ExactGeneration,
    state: snapshot.state as ConnectionState,
    errorCode: snapshot.errorCode as ConnectionErrorCode | null,
  };
}

export function acceptsConnectionSnapshot(current: ConnectionSnapshot, next: ConnectionSnapshot): boolean {
  if (current.connectionId !== next.connectionId) return false;
  const currentGeneration = BigInt(current.generation);
  const nextGeneration = BigInt(next.generation);
  if (nextGeneration < currentGeneration) return false;
  if (nextGeneration > currentGeneration) return next.state === "connecting";
  if (current.state === next.state && current.errorCode === next.errorCode) return true;
  switch (current.state) {
    case "connecting": return ["authenticating", "online", "reconnecting", "disconnected", "failed"].includes(next.state);
    case "authenticating": return ["online", "reconnecting", "disconnected", "failed"].includes(next.state);
    case "online": return ["reconnecting", "disconnected"].includes(next.state);
    case "reconnecting": return ["disconnected", "failed"].includes(next.state);
    case "failed": return next.state === "disconnected";
    case "disconnected": return false;
  }
}
