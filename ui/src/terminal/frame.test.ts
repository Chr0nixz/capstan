import { describe, expect, it } from "vitest";

import { decodeTerminalFrame } from "./frame";

function makeFrame(sequence: bigint, payload: number[]): Uint8Array {
  const bytes = new Uint8Array(20 + payload.length);
  bytes.set([0x43, 0x53, 0x54, 0x4e]);
  const view = new DataView(bytes.buffer);
  view.setBigUint64(4, sequence, true);
  view.setUint32(12, payload.length, true);
  let checksum = 0x811c9dc5;
  for (const byte of payload) {
    checksum = Math.imul(checksum ^ byte, 0x01000193) >>> 0;
  }
  view.setUint32(16, checksum, true);
  bytes.set(payload, 20);
  return bytes;
}

describe("terminal binary frames", () => {
  it("decodes sequence metadata and binary bytes", () => {
    const frame = decodeTerminalFrame(makeFrame(42n, [0, 255, 1, 0]));
    expect(frame.sequence).toBe(42n);
    expect(Array.from(frame.payload)).toEqual([0, 255, 1, 0]);
  });

  it("rejects corrupted and malformed frames", () => {
    const corrupted = makeFrame(0n, [1, 2, 3]);
    corrupted[20] = 4;
    expect(() => decodeTerminalFrame(corrupted)).toThrow(/checksum/);
    expect(() => decodeTerminalFrame(new Uint8Array([1, 2]))).toThrow(/header/);
  });

  it("rejects JSON arrays instead of silently converting the IPC fallback", () => {
    expect(() => decodeTerminalFrame(Array.from(makeFrame(0n, [1, 2, 3])))).toThrow(/raw binary/);
  });

  it("validates every byte of maximum-size frames through an offset binary view", () => {
    const payload = Array.from({ length: 64 * 1024 }, (_, index) => index & 255);
    const frame = makeFrame(99n, payload);
    const storage = new Uint8Array(frame.length + 32).fill(255);
    storage.set(frame, 13);
    const view = storage.subarray(13, 13 + frame.length);
    expect(decodeTerminalFrame(view).payload).toEqual(new Uint8Array(payload));
    for (const position of [20, 20 + 32768, view.length - 1]) {
      view[position] ^= 1;
      expect(() => decodeTerminalFrame(view)).toThrow(/checksum/);
      view[position] ^= 1;
    }
  });

  it("checks transport padding and excludes it from the terminal payload", () => {
    const padded = new Uint8Array(1024);
    padded.set(makeFrame(42n, [0, 255, 1, 0]));
    const decoded = decodeTerminalFrame(padded.buffer);
    expect(decoded.sequence).toBe(42n);
    expect(Array.from(decoded.payload)).toEqual([0, 255, 1, 0]);
    padded[1000] = 1;
    expect(() => decodeTerminalFrame(padded)).toThrow(/padding/);
    expect(() => decodeTerminalFrame(padded.subarray(0, 1023))).toThrow(/payload length/);
  });
});
