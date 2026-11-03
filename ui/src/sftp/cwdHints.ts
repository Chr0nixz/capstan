import { createStore, type StoreApi } from "zustand/vanilla";
import type { TerminalSessionId } from "../terminal/contracts";
import type { ExactU64 } from "./contracts";

export type CwdHint = { sequence: ExactU64; uri: string };
const maximumBytes = 8192;
const maximumSequence = 18446744073709551615n;

export function boundedCwdHint(uri: string): boolean {
  if (uri.length > maximumBytes) return false;
  const bytes = new TextEncoder().encode(uri);
  return bytes.length <= maximumBytes
    && new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes) === uri;
}

/** Parser callbacks only replace one bounded pending hint. They never await IPC. */
export function createCwdHintHub() {
  const slots = new Map<TerminalSessionId, {
    store: StoreApi<CwdHint | null>;
    owner: symbol | null; subscribers: number; sequence: bigint;
    pending: CwdHint | null; timer: ReturnType<typeof setTimeout> | null;
  }>();
  const slotFor = (terminal: TerminalSessionId) => {
    const existing = slots.get(terminal);
    if (existing) return existing;
    if (slots.size >= 128) return undefined;
    const slot = {
      store: createStore<CwdHint | null>(() => null), owner: null as symbol | null,
      subscribers: 0, sequence: 0n, pending: null as CwdHint | null,
      timer: null as ReturnType<typeof setTimeout> | null,
    };
    slots.set(terminal, slot);
    return slot;
  };
  const prune = (terminal: TerminalSessionId): void => {
    const slot = slots.get(terminal);
    if (slot && slot.owner === null && slot.subscribers === 0) slots.delete(terminal);
  };
  return {
    open(terminal: TerminalSessionId) {
      const slot = slotFor(terminal);
      const owner = Symbol();
      if (slot && slot.owner === null) slot.owner = owner;
      return {
        publish(uri: string): void {
          if (!slot || slot.owner !== owner || slot.sequence === maximumSequence || !boundedCwdHint(uri)) return;
          slot.sequence++;
          slot.pending = { sequence: slot.sequence.toString() as ExactU64, uri };
          slot.timer ??= setTimeout(() => {
            slot.timer = null;
            const latest = slot.pending;
            slot.pending = null;
            if (slot.owner === owner && latest) slot.store.setState(latest, true);
          }, 250);
        },
        close(): void {
          if (!slot || slot.owner !== owner) return;
          if (slot.timer !== null) clearTimeout(slot.timer);
          slot.timer = null; slot.pending = null; slot.owner = null;
          slot.store.setState(null, true);
          prune(terminal);
        },
      };
    },
    subscribe(terminal: TerminalSessionId, listener: (hint: CwdHint | null) => void): () => void {
      const slot = slotFor(terminal);
      if (!slot) return () => undefined;
      slot.subscribers++;
      const unsubscribe = slot.store.subscribe(listener);
      listener(slot.store.getState());
      return () => { unsubscribe(); slot.subscribers--; prune(terminal); };
    },
  };
}

export const cwdHints = createCwdHintHub();
