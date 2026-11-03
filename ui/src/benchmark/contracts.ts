import type { TerminalSessionId, TerminalSnapshot } from "../terminal/contracts";

export type Scenario = "bulk" | "idle";
export type SourceSample = { digest: { bytes: number; checksum: number; eof: boolean }; snapshot: TerminalSnapshot };
export type ReceiverSample = {
  id: TerminalSessionId; bytes: number; checksum: number; frames: number; raw_binary: boolean;
  parsed_bytes: number; acked_bytes: number; max_pending_bytes: number;
  bulk_elapsed_ms: number;
  input_response_ms?: number[];
};
export type Failure = "timeout" | "integrity" | "input" | "backend" | "renderer" | "cleanup" | "cancelled";
export type Run = {
  kind: "bulk" | "delayed_ack" | "tab_lifecycle"; round: number; elapsed_ms: number;
  renderer_peak: number; renderers_after: number; registered_after: number;
  input_response_ms: number[]; terminals: ReceiverSample[]; failure: Failure | null;
  isolation_input_response_ms: number[];
};
export type Report = { schema: 1; viewport: { width: number; height: number; scale: number }; runs: Run[] };
