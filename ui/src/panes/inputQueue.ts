import { InputQueue, type InputEnqueueResult } from "../terminal/inputQueue";

/** Captures routing when xterm enqueues input, rather than after a slow write
 * or confirmation has yielded. The original byte/item limits still apply. */
export class RoutedInputQueue<T> extends InputQueue {
  private readonly routes: T[] = [];

  constructor(private readonly capture: (bytes: Uint8Array) => T) { super(); }

  override enqueue(bytes: Uint8Array): InputEnqueueResult {
    const result = super.enqueue(bytes);
    if (result.accepted) this.routes.push(this.capture(bytes));
    return result;
  }

  route(): T | undefined { return this.routes[0]; }

  override dequeue(): Uint8Array | undefined {
    const bytes = super.dequeue();
    if (bytes) this.routes.shift();
    return bytes;
  }

  override clear(): void { super.clear(); this.routes.length = 0; }
}
