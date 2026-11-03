import { describe, expect, it } from "vitest";

import { InputQueue } from "./inputQueue";

describe("InputQueue", () => {
  it("keeps FIFO order and tracks bytes", () => {
    const queue = new InputQueue({ maxBytes: 8, maxItems: 4 });
    expect(queue.enqueue(new Uint8Array([1, 2])).accepted).toBe(true);
    expect(queue.enqueue(new Uint8Array([3])).accepted).toBe(true);
    expect(queue.bytes).toBe(3);
    expect(Array.from(queue.dequeue() ?? [])).toEqual([1, 2]);
    expect(Array.from(queue.dequeue() ?? [])).toEqual([3]);
    expect(queue.bytes).toBe(0);
  });

  it("rejects input instead of blocking when full", () => {
    const queue = new InputQueue({ maxBytes: 2, maxItems: 1 });
    expect(queue.enqueue(new Uint8Array([1, 2])).accepted).toBe(true);
    const result = queue.enqueue(new Uint8Array([3]));
    expect(result.accepted).toBe(false);
    expect(result.droppedBytes).toBe(1);
    expect(queue.bytes).toBe(2);
  });

  it("clears and rejects input after close", () => {
    const queue = new InputQueue();
    queue.enqueue(new Uint8Array([1]));
    queue.close();
    expect(queue.isClosed).toBe(true);
    expect(queue.bytes).toBe(0);
    expect(queue.enqueue(new Uint8Array([2])).accepted).toBe(false);
  });
});
