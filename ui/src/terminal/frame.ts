const FRAME_MAGIC = [0x43, 0x53, 0x54, 0x4e] as const;
const FRAME_HEADER_LENGTH = 20;
const MAX_FRAME_PAYLOAD = 64 * 1024;

export type TerminalFrame = {
  sequence: bigint;
  payload: Uint8Array;
};

export function decodeTerminalFrame(message: unknown): TerminalFrame {
  if (!(message instanceof ArrayBuffer) && !(message instanceof Uint8Array)) {
    throw new Error("terminal frame is not raw binary");
  }
  const bytes = message instanceof Uint8Array ? message : new Uint8Array(message);
  if (bytes.byteLength < FRAME_HEADER_LENGTH) {
    throw new Error("terminal frame is shorter than its header");
  }
  if (!FRAME_MAGIC.every((byte, index) => bytes[index] === byte)) {
    throw new Error("terminal frame has an invalid signature");
  }

  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const sequence = view.getBigUint64(4, true);
  const payloadLength = view.getUint32(12, true);
  const expectedChecksum = view.getUint32(16, true);
  const payloadEnd = FRAME_HEADER_LENGTH + payloadLength;
  // The Tauri adapter pads small frames to bypass its inline JSON optimization.
  const padded = payloadEnd < 1024 && bytes.byteLength === 1024;
  if (payloadLength > MAX_FRAME_PAYLOAD || (bytes.byteLength !== payloadEnd && !padded)) {
    throw new Error("terminal frame has an invalid payload length");
  }
  if (padded && bytes.subarray(payloadEnd).some((byte) => byte !== 0)) {
    throw new Error("terminal frame has invalid transport padding");
  }

  const payload = bytes.subarray(FRAME_HEADER_LENGTH, payloadEnd);
  if (checksum(payload) !== expectedChecksum) {
    throw new Error("terminal frame checksum mismatch");
  }
  return { sequence, payload };
}

function checksum(bytes: Uint8Array): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < bytes.length; index++) {
    hash = Math.imul(hash ^ bytes[index], 0x01000193) >>> 0;
  }
  return hash;
}
