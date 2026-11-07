import { create } from "zustand";
import { invokeTauri } from "../terminal/bridge";
import type { UpdateStatus } from "./contracts";

type UpdateStore = { status: UpdateStatus | null; dialogOpen: boolean; preferenceError: boolean; saving: boolean; loadError: boolean };
export const useUpdates = create<UpdateStore>(() => ({ status: null, dialogOpen: false, preferenceError: false, saving: false, loadError: false }));

export async function refreshUpdates(): Promise<void> {
  try { useUpdates.setState({ status: await invokeTauri("update_status"), loadError: false }); }
  catch { useUpdates.setState({ loadError: true }); }
}

export async function checkUpdates(): Promise<void> {
  const status = useUpdates.getState().status;
  if (!status?.available || ["checking", "downloading", "installing"].includes(status.phase)) return;
  useUpdates.setState({ status: { ...status, phase: "checking", errorCode: null } });
  try {
    const next = await invokeTauri("check_update");
    useUpdates.setState({ status: next, dialogOpen: next.release !== null });
  } catch { await refreshUpdates(); }
}
