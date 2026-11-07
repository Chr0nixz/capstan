import { Terminal } from "@xterm/xterm";
import { expect, it, vi } from "vitest";
import { createTerminalSession, type TerminalLike } from "./session";

type MouseEvent = { col: number; row: number; button: number; action: number; x: number; y: number };

it.each([
  { mode: "legacy", sequence: "", report: [27, 91, 77, 34, 33, 33] },
  { mode: "SGR", sequence: "\x1b[?1006h", report: [27, 91, 60, 50, 59, 49, 59, 49, 77] },
  { mode: "SGR pixels", sequence: "\x1b[?1016h", report: [27, 91, 60, 50, 59, 49, 48, 59, 50, 48, 77] },
])("routes $mode mouse reports only to the source pane", async ({ sequence, report }) => {
  const terminal = new Terminal({ allowProposedApi: true });
  const { coreMouseService } = (terminal as unknown as { _core: { coreMouseService: {
    triggerMouseEvent(event: MouseEvent): boolean;
  } } })._core;
  const originalTrigger = coreMouseService.triggerMouseEvent;
  const sendInput = vi.fn<(bytes: Uint8Array) => void>();
  const sendProtocolInput = vi.fn<(bytes: Uint8Array) => void>();
  const session = createTerminalSession(document.createElement("div"), true, {
    terminal: Object.assign(terminal, { open: () => undefined }) as unknown as TerminalLike,
    rendererFactory: () => ({ dispose() {} }),
    fitAddonFactory: () => ({ fit() {} }),
    sendInput,
    sendProtocolInput,
  });
  try {
    await new Promise<void>((resolve) => terminal.write("\x1b[?1000h" + sequence, resolve));
    expect(coreMouseService.triggerMouseEvent({ col: 0, row: 0, button: 2, action: 1, x: 10, y: 20 })).toBe(true);
    expect(sendProtocolInput.mock.calls.map(([bytes]) => [...bytes])).toEqual([report]);
    expect(sendInput).not.toHaveBeenCalled();
    terminal.input(String.fromCharCode(...report), true);
    expect(sendInput.mock.calls.map(([bytes]) => [...bytes])).toEqual([report]);
    expect(sendProtocolInput).toHaveBeenCalledTimes(1);
  } finally {
    session.dispose();
  }
  expect(coreMouseService.triggerMouseEvent).toBe(originalTrigger);
});
