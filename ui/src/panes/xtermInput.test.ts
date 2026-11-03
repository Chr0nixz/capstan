import { Terminal } from "@xterm/xterm";
import { describe, expect, it, vi } from "vitest";
import type { TerminalLike } from "../terminal/session";
import { subscribeTerminalInput, type TerminalInputOrigin } from "./xtermInput";

describe("xterm input provenance", () => {
  it("preserves real xterm DSR replies and identical user bytes as different origins", async () => {
    const terminal = new Terminal();
    const received: { data: string; origin: TerminalInputOrigin }[] = [];
    const subscription = subscribeTerminalInput(terminal as unknown as TerminalLike, (data, origin) => received.push({ data, origin }));
    try {
      await new Promise<void>((resolve) => terminal.write("\x1b[6n", resolve));
      expect(received).toEqual([{ data: "\x1b[1;1R", origin: "protocol" }]);
      terminal.input(received[0].data, true);
      expect(received[1]).toEqual({ data: "\x1b[1;1R", origin: "user" });
      terminal.input("中文\r", true);
      expect(received[2]).toEqual({ data: "中文\r", origin: "user" });
    } finally { subscription.dispose(); terminal.dispose(); }
  });

  it("restores the enclosing origin after nested dispatch and removes its hook on dispose", () => {
    let onData: (data: string) => void = () => undefined;
    const service = { triggerDataEvent: (data: string, _user = false): void => onData(data) };
    const original = service.triggerDataEvent, dispose = vi.fn();
    const terminal = { _core: { coreService: service }, onData: (listener: (data: string) => void) => { onData = listener; return { dispose }; } };
    const received: [string, TerminalInputOrigin][] = [];
    const subscription = subscribeTerminalInput(terminal as unknown as TerminalLike, (data, origin) => {
      received.push([data, origin]);
      if (data === "outer") { service.triggerDataEvent("nested", false); onData("outer-tail"); }
    });
    service.triggerDataEvent("outer", true);
    onData("unclassified");
    expect(received).toEqual([["outer", "user"], ["nested", "protocol"], ["outer-tail", "user"], ["unclassified", "unknown"]]);
    subscription.dispose(); subscription.dispose();
    expect(service.triggerDataEvent).toBe(original);
    expect(dispose).toHaveBeenCalledTimes(1);
  });

  it.each(["missing", "read-only"])("classifies a %s dispatcher as unknown instead of guessing a user origin", (kind) => {
    let onData: (data: string) => void = () => undefined;
    const terminal = { onData: (listener: (data: string) => void) => { onData = listener; return { dispose: vi.fn() }; } };
    if (kind === "read-only") Object.assign(terminal, { _core: { coreService: Object.freeze({ triggerDataEvent: (data: string): void => onData(data) }) } });
    const listener = vi.fn();
    const subscription = subscribeTerminalInput(terminal as unknown as TerminalLike, listener);
    onData("unknown");
    expect(listener).toHaveBeenCalledExactlyOnceWith("unknown", "unknown");
    subscription.dispose();
  });
});
