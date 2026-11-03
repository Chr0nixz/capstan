import { createTauriChannel, invokeTauri } from "./bridge";
import type { StartedTerminal, TerminalLaunch, TerminalSessionId, TerminalSnapshot } from "./contracts";

export interface TerminalTransport {
  start: (launch: TerminalLaunch, output: (frame: ArrayBuffer) => void, status: (snapshot: TerminalSnapshot) => void) => Promise<StartedTerminal>;
  input: (id: TerminalSessionId, bytes: Uint8Array) => Promise<void>;
  ack: (id: TerminalSessionId, bytes: number) => Promise<void>;
  resize: (id: TerminalSessionId, columns: number, rows: number) => Promise<void>;
  stop: (id: TerminalSessionId) => Promise<void>;
  cancelAttempt: (attemptId: string) => Promise<void>;
  detach: () => void;
}

export function createTauriTransport(): TerminalTransport {
  const output = createTauriChannel<ArrayBuffer>();
  const status = createTauriChannel<TerminalSnapshot>();
  return {
    async start(launch, onOutput, onStatus) {
      if (!output || !status) throw new Error("runtime_unavailable");
      output.onmessage = onOutput;
      status.onmessage = onStatus;
      if (launch.kind === "ssh") {
        return invokeTauri("start_ssh_terminal", { hostId: launch.host_id, attemptId: launch.attempt_id, reconnect: launch.reconnect, onOutput: output, onStatus: status });
      }
      return invokeTauri("start_local_terminal", { options: launch.options, onOutput: output, onStatus: status });
    },
    input: (id, bytes) => invokeTauri("write_terminal_input", { id, input: new TextDecoder().decode(bytes) }),
    ack: (id, bytes) => invokeTauri("ack_terminal_output", { id, bytes }),
    resize: (id, columns, rows) => invokeTauri("resize_terminal", { id, columns, rows }),
    stop: (id) => invokeTauri("stop_terminal", { id }),
    cancelAttempt: (attemptId) => invokeTauri("cancel_ssh_attempt", { attemptId }),
    detach() {
      if (output) output.onmessage = null;
      if (status) status.onmessage = null;
    },
  };
}
