import { describe, expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-sftp.json";
import { decodeDirectoryPage, decodeExactU64, decodeShellPreview } from "./contracts";

describe("SFTP metadata contract", () => {
  it("reads the same fixture as Rust without losing 64-bit integers", () => {
    const page = decodeDirectoryPage(fixture);
    expect(page).toEqual(fixture);
    expect(page.entries[0].metadata.size).toBe("18446744073709551615");
    expect(BigInt(page.requestRevision)).toBe(9_007_199_254_740_993n);
    expect(page.entries[1].metadata.size).toBeNull();
  });

  it("accepts only canonical decimal strings in the u64 range", () => {
    for (const input of ["0", "9007199254740993", "18446744073709551615"]) expect(decodeExactU64(input)).toBe(input);
    for (const input of [0, 9_007_199_254_740_992, null, "", "01", "+1", "-1", "1.0", "1e3", " 1", "18446744073709551616"]) {
      expect(() => decodeExactU64(input)).toThrow("sftp_contract_invalid");
    }
  });

  it("rejects malformed schema, excess entries and unsafe names", () => {
    const entry = fixture.entries[0];
    for (const input of [
      { ...fixture, sessionId: "forged" },
      { ...fixture, requestRevision: 1 },
      { ...fixture, path: "relative" },
      { ...fixture, path: "/bad\0path" },
      { ...fixture, path: "/bad\ud800" },
      { ...fixture, path: "/" + "a".repeat(4096) },
      { ...fixture, hasMore: 0 },
      { ...fixture, omittedEntries: -1 },
      { ...fixture, omittedEntries: 0x1_0000_0000 },
      { ...fixture, pageSequence: -1 },
      { ...fixture, pageSequence: 1.5 },
      { ...fixture, pageSequence: 0x1_0000_0000 },
      { ...fixture, extra: true },
      { ...fixture, entries: Array.from({ length: 257 }, () => entry) },
      { ...fixture, entries: [entry, entry] },
      ...["", ".", "..", "a/b", "bad\ufffd", "a\nb", "bad\udfff", "中".repeat(86)].map((name) => ({ ...fixture, entries: [{ ...entry, name }] })),
      { ...fixture, entries: [{ ...entry, metadata: { ...entry.metadata, size: 1 } }] },
      { ...fixture, entries: [{ ...entry, metadata: { ...entry.metadata, permissions: -1 } }] },
      { ...fixture, entries: [{ ...entry, metadata: { ...entry.metadata, kind: "device" } }] },
      { ...fixture, entries: [{ ...entry, metadata: { ...entry.metadata, extra: true } }] },
    ]) expect(() => decodeDirectoryPage(input)).toThrow("sftp_contract_invalid");
  });

  it("keeps manual paths literal, including percent escapes and remote symlink traversal", () => {
    const path = "/link/../中文/%2f\\literal";
    expect(decodeDirectoryPage({ ...fixture, path }).path).toBe(path);
    for (const name of ["\ufeffliteral-name", "emoji-😀"]) {
      expect(decodeDirectoryPage({ ...fixture, entries: [{ ...fixture.entries[0], name }] }).entries[0].name).toBe(name);
    }
  });

  it("validates shell preview metadata without accepting configuration bytes", () => {
    const preview = decodeShellPreview({
      token: "00000000-0000-4000-8000-000000000010", shell: "bash", action: "install", host: "example.com",
      sourceHash: "A".repeat(64), replacementHash: "B".repeat(64), addedLines: 12, removedLines: 0,
    });
    expect(preview.addedLines).toBe(12);
    for (const invalid of [
      { ...preview, sourceHash: "not-a-hash" },
      { ...preview, action: "remove" },
      { ...preview, addedLines: 65537 },
      { ...preview, host: "bad\0host" },
      { ...preview, body: "must never cross the boundary" },
    ]) expect(() => decodeShellPreview(invalid)).toThrow("sftp_contract_invalid");
  });
});
