import { invokeTauri, listenTauriEvent } from "../terminal/bridge";
import { acceptsConnectionSnapshot, connectionStateChangedEvent, decodeConnectionSnapshot, type ConnectionSnapshot } from "./contracts";

export type ConnectionSnapshotSource = "event" | "query";

/** Subscribe before querying; an authoritative query can fill missed generations. */
export async function watchConnections(receive: (snapshot: ConnectionSnapshot, source: ConnectionSnapshotSource) => void): Promise<() => void> {
  let disposed = false;
  const current = new Map<string, ConnectionSnapshot>();
  const apply = (value: unknown, authoritative = false): boolean => {
    if (disposed) return true;
    let next: ConnectionSnapshot;
    try { next = decodeConnectionSnapshot(value); } catch { return true; }
    const previous = current.get(next.connectionId);
    if (previous?.generation === next.generation && previous.state === next.state && previous.errorCode === next.errorCode) return true;
    if (previous && !acceptsConnectionSnapshot(previous, next)
      && !(authoritative && BigInt(next.generation) > BigInt(previous.generation))) return BigInt(next.generation) <= BigInt(previous.generation);
    current.set(next.connectionId, next);
    receive(next, authoritative ? "query" : "event");
    return true;
  };
  const unlisten = await listenTauriEvent<unknown>(connectionStateChangedEvent, (value) => {
    // If IPC delivery skipped a connecting event, recover from a fresh snapshot.
    if (!apply(value)) void invokeTauri("connection_snapshots").then((snapshots) => snapshots.forEach((snapshot) => apply(snapshot, true))).catch(() => undefined);
  });
  try {
    const snapshots = await invokeTauri("connection_snapshots");
    snapshots.forEach((snapshot) => apply(snapshot, true));
  } catch (error) { disposed = true; unlisten(); throw error; }
  return () => { disposed = true; unlisten(); };
}
