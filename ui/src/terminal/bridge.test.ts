import { afterEach, describe, expect, it, vi } from "vitest";
import { writeTerminalInput } from "./bridge";
import { createTauriTransport } from "./transport";
import type { TerminalSessionId } from "./contracts";

afterEach(() => { delete window.__TAURI__; });

describe("terminal input", () => {
  it("sends the bytes themselves, so a mouse report past column 95 survives", async () => {
    const invoke = vi.fn(async (_command: string, _args?: unknown, _options?: unknown): Promise<unknown> => undefined);
    window.__TAURI__ = { core: { invoke: <T,>(command: string, args?: unknown, options?: unknown) => invoke(command, args, options) as Promise<T>, Channel: class { onmessage = null; } } };
    // X10 mouse press at column 200: ESC [ M, button, x + 32, y + 32. 232 is not UTF-8 on its own.
    const report = Uint8Array.of(0x1b, 0x5b, 0x4d, 32, 232, 33);
    await createTauriTransport().input("session-1" as TerminalSessionId, report);
    expect(invoke).toHaveBeenCalledWith("write_terminal_input", report, { headers: { "capstan-terminal": "session-1" } });
    const body = invoke.mock.calls[0]?.[1] as Uint8Array;
    expect([...body]).toEqual([0x1b, 0x5b, 0x4d, 32, 232, 33]);
  });

  it("fails without the runtime instead of dropping input silently", async () => {
    await expect(writeTerminalInput("session-1", Uint8Array.of(97))).rejects.toThrow("Tauri runtime unavailable");
  });
});
