import { create } from "zustand";

/** Logical connection identities learned exclusively from successful shell startup. */
export const useTunnelTargets = create<{ hosts: Readonly<Record<string, string>>; register: (host: string, connection: string) => void }>((set) => ({
  hosts: {},
  register: (host, connection) => set((state) => state.hosts[host] === connection ? state : { hosts: { ...state.hosts, [host]: connection } }),
}));
