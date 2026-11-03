/** Streaming counters only; terminal contents are never part of a report. */
export class ReceiverLedger {
  bytes = 0;
  parsedBytes = 0;
  ackedBytes = 0;
  checksum = 0x811c9dc5;
  frames = 0;
  rawBinary = true;
  maxPendingBytes = 0;

  receive(bytes: Uint8Array): void {
    for (let index = 0; index < bytes.length; index++) this.checksum = Math.imul(this.checksum ^ bytes[index], 0x01000193) >>> 0;
    this.bytes += bytes.byteLength;
    this.frames += 1;
    this.maxPendingBytes = Math.max(this.maxPendingBytes, this.bytes - this.parsedBytes);
  }
  parsed(bytes: Uint8Array): void { this.parsedBytes += bytes.byteLength; }
  acknowledge(bytes: number): void { this.ackedBytes += bytes; }
}

/** Recognizes fixed fixture markers across read boundaries and ANSI controls. */
export class ProbeMarkers {
  private state: "text" | "escape" | "csi" | "osc" | "osc_escape" = "text";
  private prefix = 0;
  private suffix: "choose" | "end" | "probe" | "digits" = "choose";
  private matched = 0;
  private sequence = 0;
  ended = false;
  constructor(private readonly onProbe: (sequence: number) => void) {}

  accept(bytes: Uint8Array): void {
    for (let index = 0; index < bytes.length; index++) {
      const byte = bytes[index];
      if (this.state === "escape") {
        this.state = byte === 0x5b ? "csi" : byte === 0x5d ? "osc" : "text";
        continue;
      }
      if (this.state === "csi") { if (byte >= 0x40 && byte <= 0x7e) this.state = "text"; continue; }
      if (this.state === "osc") { if (byte === 7) this.state = "text"; else if (byte === 0x1b) this.state = "osc_escape"; continue; }
      if (this.state === "osc_escape") { this.state = byte === 0x5c ? "text" : "osc"; continue; }
      if (byte === 0x1b) { this.state = "escape"; continue; }
      // Seek a fixed uppercase prefix without per-byte strings or regex work.
      const prefix = "CAPSTAN_M0_";
      if (this.prefix < prefix.length) {
        this.prefix = byte === prefix.charCodeAt(this.prefix) ? this.prefix + 1 : byte === 67 ? 1 : 0;
        continue;
      }
      if (this.suffix === "choose") {
        this.suffix = byte === 69 ? "end" : byte === 80 ? "probe" : "choose";
        this.matched = 0;
        if (this.suffix === "choose") this.prefix = 0;
        continue;
      }
      if (this.suffix === "digits") {
        if (byte >= 48 && byte <= 57) {
          this.sequence = this.sequence * 10 + byte - 48;
          this.matched += 1;
          if (this.matched === 6) { this.onProbe(this.sequence); this.prefix = 0; this.suffix = "choose"; }
        } else { this.prefix = 0; this.suffix = "choose"; }
        continue;
      }
      const suffix = this.suffix === "end" ? "ND" : "ROBE:";
      if (byte === suffix.charCodeAt(this.matched)) {
        this.matched += 1;
        if (this.matched === suffix.length) {
          if (this.suffix === "end") { this.ended = true; this.prefix = 0; this.suffix = "choose"; }
          else { this.suffix = "digits"; this.matched = 0; this.sequence = 0; }
        }
      } else { this.prefix = 0; this.suffix = "choose"; }
    }
  }
}

export function percentile(values: number[], fraction: number): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.ceil(fraction * sorted.length) - 1)] ?? null;
}
