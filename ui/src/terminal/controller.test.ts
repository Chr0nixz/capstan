import { describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/terminal-contract.json";
import { createTerminalController } from "./controller";
import type { HostId, StartedTerminal, TerminalLaunch, TerminalSessionId, TerminalSnapshot } from "./contracts";
import type { TerminalTransport } from "./transport";
import { decodeTerminalFrame } from "./frame";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function setup(launch: TerminalLaunch = { kind: "local", options: {} }) {
  const started = deferred<StartedTerminal>();
  let output!: (frame: ArrayBuffer) => void;
  let status!: (snapshot: TerminalSnapshot) => void;
  const transport: TerminalTransport = {
    start: vi.fn((_launch, onOutput, onStatus) => { output = onOutput; status = onStatus; return started.promise; }),
    input: vi.fn(async () => undefined), ack: vi.fn(async () => undefined),
    resize: vi.fn(async () => undefined), stop: vi.fn(async () => undefined),
    cancelAttempt: vi.fn(async () => undefined), detach: vi.fn(),
  };
  const callbacks = { started: vi.fn(), output: vi.fn(), status: vi.fn(), failure: vi.fn() };
  const controller = createTerminalController(launch, callbacks, transport);
  const result: StartedTerminal = {
    kind: "local",
    id: "session-0" as TerminalSessionId,
    runtime: { source: "System", path: null, version: null, architecture: "x86_64", reason: "system" },
  };
  const frame = (sequence = 0): ArrayBuffer => {
    const bytes = new Uint8Array(fixture.encoded);
    new DataView(bytes.buffer).setBigUint64(4, BigInt(sequence), true);
    return bytes.buffer;
  };
  const snapshot = (state: TerminalSnapshot["state"], outputBytes: number): TerminalSnapshot => ({
    ...fixture.snapshot, id: result.id, state, exit_code: state === "exited" ? 0 : null,
    metrics: { ...fixture.snapshot.metrics, output_bytes: outputBytes },
  });
  return { controller, callbacks, transport, started, result, frame, snapshot, output: (buffer: ArrayBuffer) => output(buffer), status: (next: TerminalSnapshot) => status(next) };
}

describe("terminal controller contracts", () => {
  it("binds the source identity before releasing pre-start parser output", async () => {
    const test = setup();
    test.output(test.frame());
    test.callbacks.output.mockImplementation(() => expect(test.callbacks.started).toHaveBeenCalledExactlyOnceWith(test.result));
    test.started.resolve(test.result);
    await test.controller.ready;
    expect(test.callbacks.output).toHaveBeenCalledTimes(1);
    await test.controller.dispose();
  });
  it("reports an invalid local directory with an actionable message", async () => {
    const test = setup();
    test.started.reject({ code: "working_directory_invalid" });
    await test.controller.ready;
    expect(test.callbacks.failure).toHaveBeenCalledExactlyOnceWith("workingDirectoryInvalid");
    await test.controller.dispose();
  });
  it.each([
    ["host_key_changed", "hostKeyChanged"],
    ["ssh_authentication", "authenticationFailed"],
    ["host_key_rejected", "operationCanceled"],
    ["canceled", "operationCanceled"],
    ["cancelled", "operationCanceled"],
    ["ssh_timeout", "connectionTimedOut"],
    ["ssh_channel_rejected", "shellRejected"],
    ["ssh_private_key_invalid", "privateKeyInvalid"],
    ["credential_file", "credentialFileInvalid"],
    ["ssh_agent_unavailable", "agentUnavailable"],
    ["ssh_agent_identity_unavailable", "agentIdentityUnavailable"],
    ["vault_locked", "vaultLocked"],
    ["invalid_input", "invalidInput"],
    ["ssh_connection", "connectionFailed"],
    ["ssh_protocol", "connectionFailed"],
    ["backend", "connectionFailed"],
    ["unrecognized-secret-marker", "connectionFailed"],
  ])("reports SSH startup %s without a local-shell error or backend details", async (code, label) => {
    const test = setup({ kind: "ssh", host_id: "host-test" as HostId, attempt_id: "attempt-test", theme: "graphite" });
    test.started.reject({ code, message: "private-backend-detail" });
    await test.controller.ready;
    expect(test.callbacks.failure).toHaveBeenCalledExactlyOnceWith(label);
    await expect(test.controller.sendInput(new Uint8Array([1]))).rejects.toThrow("session_closed");
    expect(test.transport.input).not.toHaveBeenCalled();
    await test.controller.dispose();
  });

  it.each(["local", "ssh"] as const)("uses the correct %s startup fallback for an unstructured error", async (kind) => {
    const test = setup(kind === "ssh"
      ? { kind, host_id: "host-test" as HostId, attempt_id: "attempt-test", theme: "graphite" }
      : { kind, options: {} });
    test.started.reject(new Error("private-backend-detail"));
    await test.controller.ready;
    expect(test.callbacks.failure).toHaveBeenCalledExactlyOnceWith(kind === "ssh" ? "connectionFailed" : "startFailed");
    await test.controller.dispose();
  });

  it("decodes the same golden frame that Rust encodes", () => {
    const frame = decodeTerminalFrame(new Uint8Array(fixture.encoded));
    expect(frame.sequence).toBe(BigInt(fixture.sequence));
    expect([...frame.payload]).toEqual(fixture.payload);
  });

  it("retains early output and reports exit after buffered output", async () => {
    const test = setup();
    test.output(test.frame());
    test.status({ ...fixture.snapshot, id: test.result.id, state: "exited", exit_code: 7 });
    expect(test.callbacks.output).not.toHaveBeenCalled();
    test.started.resolve(test.result);
    await test.controller.ready;
    expect(test.callbacks.output).toHaveBeenCalledWith(new Uint8Array(fixture.payload));
    expect(test.callbacks.output.mock.invocationCallOrder[0]).toBeLessThan(test.callbacks.status.mock.invocationCallOrder[0]);
    await expect(test.controller.sendInput(new Uint8Array([1]))).rejects.toThrow("session_closed");
    await test.controller.dispose();
  });

  it("reports a clean exit only after the output it accounts for has arrived", async () => {
    const test = setup();
    test.started.resolve(test.result);
    await test.controller.ready;
    // Two 4-byte frames are still in flight when the inline status overtakes them.
    test.status(test.snapshot("exited", 8));
    expect(test.controller.ended).toBe(true);
    expect(test.callbacks.status).not.toHaveBeenCalled();
    test.output(test.frame(0));
    expect(test.callbacks.status).not.toHaveBeenCalled();
    test.output(test.frame(1));
    expect(test.callbacks.status).toHaveBeenCalledTimes(1);
    expect(test.callbacks.output.mock.invocationCallOrder[1]).toBeLessThan(test.callbacks.status.mock.invocationCallOrder[0]);
    await test.controller.dispose();
  });

  it("holds an early exit until output that arrives after startup", async () => {
    const test = setup();
    test.status(test.snapshot("exited", 4));
    test.started.resolve(test.result);
    await test.controller.ready;
    expect(test.callbacks.status).not.toHaveBeenCalled();
    test.output(test.frame(0));
    expect(test.callbacks.status).toHaveBeenCalledWith(expect.objectContaining({ state: "exited" }));
    await test.controller.dispose();
  });

  it("reports error states immediately because their output may never arrive", async () => {
    const test = setup();
    test.started.resolve(test.result);
    await test.controller.ready;
    test.status(test.snapshot("error", 8));
    expect(test.callbacks.status).toHaveBeenCalledWith(expect.objectContaining({ state: "error" }));
    await test.controller.dispose();
  });

  it("closes a session that finishes starting after unmount exactly once", async () => {
    const test = setup();
    const disposed = test.controller.dispose();
    test.output(test.frame());
    test.started.resolve(test.result);
    await disposed;
    await test.controller.dispose();
    expect(test.transport.stop).toHaveBeenCalledTimes(1);
    expect(test.callbacks.output).not.toHaveBeenCalled();
    expect(test.transport.detach).toHaveBeenCalled();
  });

  it("rejects wrong frame sequences and cancels the backend", async () => {
    const test = setup();
    test.started.resolve(test.result);
    await test.controller.ready;
    test.output(test.frame(1));
    expect(test.callbacks.failure).toHaveBeenCalledWith("outputInvalid");
    expect(test.transport.stop).toHaveBeenCalledWith(test.result.id);
    await test.controller.dispose();
  });

  it("serializes ACKs and propagates delivery failures", async () => {
    const test = setup();
    const ack = deferred<void>();
    vi.mocked(test.transport.ack).mockImplementationOnce(() => ack.promise);
    test.started.resolve(test.result);
    await test.controller.ready;
    test.controller.acknowledge(10);
    test.controller.acknowledge(20);
    expect(test.transport.ack).toHaveBeenCalledTimes(1);
    ack.resolve();
    await Promise.resolve();
    expect(test.transport.ack).toHaveBeenLastCalledWith(test.result.id, 20);
    await Promise.resolve();
    vi.mocked(test.transport.ack).mockRejectedValueOnce(new Error("ack_failed"));
    test.controller.acknowledge(1);
    await Promise.resolve();
    expect(test.callbacks.failure).toHaveBeenCalledWith("operationFailed");
    await test.controller.dispose();
  });
});
