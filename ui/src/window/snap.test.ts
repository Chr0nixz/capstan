import { describe, expect, it, vi } from "vitest";
import { createSnapController, type SnapTransport } from "./snap";

function fixture() {
  const button = document.createElement("button");
  let x = 1190.2;
  button.getBoundingClientRect = () => ({ left: x, top: 0.8, width: 46, height: 35 } as DOMRect);
  const listeners = new Map<string, () => void>();
  const unlisten = vi.fn();
  const transport: SnapTransport = {
    update: vi.fn(async () => undefined), detach: vi.fn(async () => undefined),
    listen: vi.fn(async (name, callback) => { listeners.set(name, callback); return unlisten; }),
  };
  return { button, transport, listeners, unlisten, move: () => { x -= 100; } };
}

describe("native snap hitbox ownership", () => {
  it("uses measured fractional geometry, supports hover and clears listeners", async () => {
    const f = fixture();
    const failure = vi.fn();
    const snap = createSnapController(f.button, failure, f.transport);
    await Promise.resolve();
    expect(f.transport.update).toHaveBeenCalledWith(expect.objectContaining({ x: 1190.2, y: 0.8, width: 46, height: 35 }));
    f.listeners.get("tauri-snap://snap/mouseenter")?.();
    expect(f.button.classList.contains("is-hovered")).toBe(true);
    f.listeners.get("tauri-snap://snap/mouseleave")?.();
    expect(f.button.classList.contains("is-hovered")).toBe(false);
    await snap.dispose();
    await snap.dispose();
    expect(f.transport.detach).toHaveBeenCalledTimes(1);
    expect(f.unlisten).toHaveBeenCalledTimes(2);
    expect(failure).not.toHaveBeenCalled();
    f.move(); window.dispatchEvent(new Event("resize"));
    expect(f.transport.update).toHaveBeenCalledTimes(1);
  });

  it("waits for an in-progress native update before detaching", async () => {
    const f = fixture();
    let release!: () => void;
    f.transport.update = vi.fn(() => new Promise<void>((resolve) => { release = resolve; }));
    const snap = createSnapController(f.button, vi.fn(), f.transport);
    await Promise.resolve();
    f.move(); snap.update(); // Coalesced geometry must not recreate after disposal.
    const disposal = snap.dispose();
    await Promise.resolve();
    expect(f.transport.detach).not.toHaveBeenCalled();
    release(); await disposal;
    expect(f.transport.update).toHaveBeenCalledTimes(1);
    expect(f.transport.detach).toHaveBeenCalledTimes(1);
  });

  it("reports failure once and restores DOM operation", async () => {
    const f = fixture(); const failure = vi.fn();
    f.transport.update = vi.fn(async () => { throw new Error("snap_native"); });
    const snap = createSnapController(f.button, failure, f.transport);
    await vi.waitFor(() => {
      expect(failure).toHaveBeenCalledTimes(1);
      expect(f.transport.detach).toHaveBeenCalledTimes(1);
    });
    snap.update();
    expect(f.transport.update).toHaveBeenCalledTimes(1);
    await snap.dispose();
  });

  it("a StrictMode handoff cannot detach the replacement controller's hitbox", async () => {
    const f = fixture();
    let release!: () => void;
    f.transport.update = vi.fn().mockImplementationOnce(() => new Promise<void>((resolve) => { release = resolve; }))
      .mockResolvedValue(undefined);
    const first = createSnapController(f.button, vi.fn(), f.transport);
    await Promise.resolve();
    const previousDisposal = first.dispose();
    const second = createSnapController(f.button, vi.fn(), f.transport);
    release(); await previousDisposal;
    expect(f.transport.detach).not.toHaveBeenCalled();
    await second.dispose();
    expect(f.transport.detach).toHaveBeenCalledTimes(1);
  });

  it("releases an installed hitbox when a late hover subscription fails", async () => {
    const f = fixture(); const failure = vi.fn();
    let rejectSubscription!: (reason: Error) => void;
    f.transport.listen = vi.fn().mockImplementationOnce(() => new Promise<() => void>((_resolve, reject) => {
      rejectSubscription = reject;
    })).mockResolvedValue(f.unlisten);
    const snap = createSnapController(f.button, failure, f.transport);
    await vi.waitFor(() => expect(f.transport.update).toHaveBeenCalledTimes(1));
    rejectSubscription(new Error("snap_event_unavailable"));
    await vi.waitFor(() => {
      expect(failure).toHaveBeenCalledTimes(1);
      expect(f.transport.detach).toHaveBeenCalledTimes(1);
    });
    f.move(); snap.update();
    expect(f.transport.update).toHaveBeenCalledTimes(1);
    await snap.dispose();
    expect(f.unlisten).toHaveBeenCalledTimes(1);
  });
});
