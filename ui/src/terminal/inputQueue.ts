export type InputEnqueueResult = {
  accepted: boolean;
  queuedBytes: number;
  droppedBytes: number;
};

export type InputQueueOptions = {
  maxBytes?: number;
  maxItems?: number;
};

const DEFAULT_MAX_BYTES = 64 * 1024;
const DEFAULT_MAX_ITEMS = 256;

/** A bounded, non-blocking keyboard queue for the terminal frontend. */
export class InputQueue {
  private readonly maxBytes: number;
  private readonly maxItems: number;
  private readonly items: Uint8Array[] = [];
  private queuedBytes = 0;
  private closed = false;

  constructor(options: InputQueueOptions = {}) {
    this.maxBytes = Math.max(1, options.maxBytes ?? DEFAULT_MAX_BYTES);
    this.maxItems = Math.max(1, options.maxItems ?? DEFAULT_MAX_ITEMS);
  }

  enqueue(data: Uint8Array): InputEnqueueResult {
    if (this.closed || data.byteLength === 0) {
      return { accepted: false, queuedBytes: this.queuedBytes, droppedBytes: data.byteLength };
    }
    if (
      data.byteLength > this.maxBytes - this.queuedBytes ||
      this.items.length >= this.maxItems
    ) {
      return { accepted: false, queuedBytes: this.queuedBytes, droppedBytes: data.byteLength };
    }
    this.items.push(data.slice());
    this.queuedBytes += data.byteLength;
    return { accepted: true, queuedBytes: this.queuedBytes, droppedBytes: 0 };
  }

  dequeue(): Uint8Array | undefined {
    const item = this.items.shift();
    if (item) {
      this.queuedBytes -= item.byteLength;
    }
    return item;
  }

  peek(): Uint8Array | undefined {
    return this.items[0];
  }

  drain(maxBytes = Number.MAX_SAFE_INTEGER): Uint8Array[] {
    const drained: Uint8Array[] = [];
    let bytes = 0;
    while (this.items.length > 0) {
      const next = this.items[0];
      if (bytes > 0 && bytes + next.byteLength > maxBytes) {
        break;
      }
      const item = this.dequeue();
      if (!item) {
        break;
      }
      drained.push(item);
      bytes += item.byteLength;
    }
    return drained;
  }

  close(): void {
    this.closed = true;
    this.clear();
  }

  reopen(): void {
    this.clear();
    this.closed = false;
  }

  clear(): void {
    this.items.length = 0;
    this.queuedBytes = 0;
  }

  get bytes(): number {
    return this.queuedBytes;
  }

  get size(): number {
    return this.items.length;
  }

  get isClosed(): boolean {
    return this.closed;
  }
}
