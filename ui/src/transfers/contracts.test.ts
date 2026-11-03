import { describe, expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-transfer.json";
import { decodeSelection, decodeTransferRecovery, decodeTransfers } from "./contracts";
import { formatBytes } from "./TransferQueue";
describe("transfer metadata boundary", () => {
  it("shares the exact Rust fixture and preserves large offsets", () => {
    expect(decodeTransfers(fixture)).toEqual(fixture);
    expect(BigInt(decodeTransfers(fixture)[0].total)).toBe(18446744073709551615n);
    expect(formatBytes("9007199254740993")).toBe("8.0 PiB");
    expect(formatBytes("0")).toBe("0 B");
  });
  it("rejects duplicate identities, invalid states, precision loss and file bodies", () => {
    const task = fixture[0];
    for (const input of [
      [task, task], Array(1025).fill(task), [{ ...task, contents: "body" }], [{ ...task, version: 1 }],
      [{ ...task, processed: "18446744073709551616" }], [{ ...task, total: "0" }], [{ ...task, state: "fake" }],
      [{ ...task, state: "completed" }], [{ ...task, conflict: { ...task.conflict, replaceAllowed: "yes" } }],
      [{ ...task, error: "untrusted-server-text" }], [{ ...task, remotePath: "relative" }],
    ]) expect(() => decodeTransfers(input)).toThrow();
  });
  it("accepts canceled pickers and opaque selections without paths", () => {
    expect(decodeSelection(null)).toBeNull();
    expect(decodeSelection({ token: fixture[0].id, name: "file" })).toEqual({ token: fixture[0].id, name: "file" });
    expect(() => decodeSelection({ token: fixture[0].id, name: "file", path: "C:\\secrets" })).toThrow();
  });
  it("validates bounded recovery metadata without exposing local paths", () => {
    const snapshot = { entries: [{ id: fixture[0].id, hostId: fixture[0].hostId, name: "report.bin", remotePath: "/drop/report.bin",
      direction: "upload", total: "10", partialSize: null, localPartial: false, state: "awaiting_reconnect", updatedSeconds: "1" }], incomplete: true };
    expect(decodeTransferRecovery(snapshot)).toEqual(snapshot);
    const legacy = { ...snapshot, entries: [{ ...snapshot.entries[0], state: "manual_inspection" }] };
    expect(decodeTransferRecovery(legacy)).toEqual(legacy);
    const cleanup = { ...snapshot, entries: [{ ...snapshot.entries[0], state: "cleanup_required" }] };
    expect(decodeTransferRecovery(cleanup)).toEqual(cleanup);
    expect(() => decodeTransferRecovery({ ...legacy, entries: [{ ...legacy.entries[0], policySha256: "a".repeat(64) }] })).toThrow();
    expect(() => decodeTransferRecovery({ ...snapshot, entries: [{ ...snapshot.entries[0], localPath: "C:\\secret" }] })).toThrow();
    expect(() => decodeTransferRecovery({ ...snapshot, entries: [{ ...snapshot.entries[0], state: "unknown" }] })).toThrow();
    expect(() => decodeTransferRecovery({ ...snapshot, entries: Array(129).fill(snapshot.entries[0]) })).toThrow();
  });
  it("bounds folder metadata and admits batch actions only for a checked existing target", () => {
    const task = { ...fixture[0], batch: { id: fixture[0].hostId, name: "Folder", files: 5, conflicts: 4 }, conflict: { ...fixture[0].conflict, batchEligible: true } };
    expect(decodeTransfers([task])[0]).toEqual(task);
    for (const input of [
      { ...task, batch: null }, { ...task, batch: { ...task.batch, files: 1025 } },
      { ...task, batch: { ...task.batch, files: 0 } }, { ...task, batch: { ...task.batch, conflicts: 6 } },
      { ...task, batch: { ...task.batch, conflicts: 0 } }, { ...task, batch: { ...task.batch, files: 1.5 } },
      { ...task, batch: { ...task.batch, path: "C:\\secrets" } }, { ...task, conflict: { ...task.conflict, target: null } },
      { ...task, conflict: { ...task.conflict, batchEligible: "yes" } },
    ]) expect(() => decodeTransfers([input])).toThrow();
  });
});
