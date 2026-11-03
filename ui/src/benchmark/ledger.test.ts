import { describe, expect, it } from "vitest";
import { ProbeMarkers, ReceiverLedger, percentile } from "./ledger";
import fixture from "../../../docs/agent/fixtures/terminal-contract.json";

describe("synthetic benchmark evidence", () => {
  it("keeps the same digest across arbitrary binary frame boundaries", () => {
    const ledger = new ReceiverLedger();
    for (const byte of fixture.payload) ledger.receive(new Uint8Array([byte]));
    expect(ledger.checksum).toBe(new DataView(new Uint8Array(fixture.encoded).buffer).getUint32(16, true));
    expect(ledger.bytes).toBe(4);
    expect(ledger.maxPendingBytes).toBe(4);
    ledger.parsed(new Uint8Array(4)); ledger.acknowledge(4);
    expect(ledger.parsedBytes).toBe(ledger.ackedBytes);
    expect(Object.values(ledger).every((value) => typeof value !== "object")).toBe(true);
  });

  it("detects end/probe markers split by frames and ANSI without retaining lines", () => {
    const probes: number[] = [];
    const parser = new ProbeMarkers((sequence) => probes.push(sequence));
    const text = "\u001b]0;CAPSTAN_M0_PROBE:999999\u0007\r\nCAPSTAN_\u001b[32mM0_PROBE:000012\r\nCAPSTAN_M0_END\r\n";
    for (const byte of new TextEncoder().encode(text)) parser.accept(new Uint8Array([byte]));
    expect(probes).toEqual([12]);
    expect(parser.ended).toBe(true);
  });

  it("records raw samples and computes nearest-rank median/high percentile", () => {
    const samples = [9, 1, 4, 2, 8];
    expect(percentile(samples, 0.5)).toBe(4);
    expect(percentile(samples, 0.95)).toBe(9);
    expect(percentile([], 0.95)).toBeNull();
    expect(samples).toEqual([9, 1, 4, 2, 8]);
  });

  it("counts offset binary views and finds the same control-aware markers at every chunk size", () => {
    const bytes = new TextEncoder().encode("a".repeat(65536) + "\u001b]title;CAPSTAN_M0_PROBE:999999\u0007"
      + "CAPSTAN_M0_PROBE:000007\r\n\u001b[32mCAPSTAN_M0_PROBE:000042\r\nCAPSTAN_M0_END");
    const whole = new ReceiverLedger();
    whole.receive(bytes);
    for (const chunk of [1, 7, 4096, 65536]) {
      const ledger = new ReceiverLedger();
      const seen: number[] = [];
      const markers = new ProbeMarkers(sequence => seen.push(sequence));
      for (let offset = 0; offset < bytes.length; offset += chunk) {
        const view = bytes.subarray(offset, offset + chunk);
        ledger.receive(view);
        markers.accept(view);
      }
      expect([ledger.bytes, ledger.checksum]).toEqual([bytes.length, whole.checksum]);
      expect(seen).toEqual([7, 42]);
      expect(markers.ended).toBe(true);
    }
  });
});
