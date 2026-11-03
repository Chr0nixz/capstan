import { describe, expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-delete.json";
import { decodeDeleteTreeReport, decodeDeleteTreeSummary } from "./treeDelete";

describe("recursive deletion metadata boundary", () => {
  it("decodes the shared Rust scope and partial results", () => {
    expect(decodeDeleteTreeSummary(fixture.summary)).toEqual(fixture.summary);
    expect(decodeDeleteTreeReport(fixture.report)).toEqual(fixture.report);
  });
  it("rejects forged identities, unbounded counts, duplicate paths and contradictory results", () => {
    for (const value of [{ ...fixture.summary, files: 4096 }, { ...fixture.summary, token: "fake" }, { ...fixture.summary, directories: 0 }]) expect(() => decodeDeleteTreeSummary(value)).toThrow();
    for (const entries of [[fixture.report.entries[0], fixture.report.entries[0]], [{ ...fixture.report.entries[0], error: "remote_io" }], [{ ...fixture.report.entries[1], path: "/bad\npath" }]]) expect(() => decodeDeleteTreeReport({ entries })).toThrow();
  });
});
