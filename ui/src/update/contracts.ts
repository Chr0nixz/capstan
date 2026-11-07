export type UpdatePhase = "idle" | "checking" | "available" | "downloading" | "installing" | "error";
export type UpdateRelease = { id: string; version: string; notes: string };
export type UpdateStatus = {
  available: boolean;
  portable: boolean;
  checkOnStartup: boolean;
  phase: UpdatePhase;
  release: UpdateRelease | null;
  downloaded: number;
  total: number;
  errorCode: string | null;
};
