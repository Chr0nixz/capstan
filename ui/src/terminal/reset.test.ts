import { Terminal } from "@xterm/xterm";
import { expect, it } from "vitest";
import { RECONNECT_RESET } from "./reset";

const write = (terminal: Terminal, data: string): Promise<void> => new Promise((resolve) => terminal.write(data, resolve));

it("leaves no mode of an interrupted full-screen program behind after a reconnect", async () => {
  const terminal = new Terminal({ allowProposedApi: true });
  const { coreMouseService } = (terminal as unknown as { _core: { coreMouseService: {
    activeProtocol: string;
    activeEncoding: string;
  } } })._core;
  try {
    // What vim or tmux with the mouse on leaves set, and a sequence cut off mid-way.
    await write(terminal, [
      "\x1b[?1049h", "\x1b[?1003h", "\x1b[?1006h", "\x1b[?1004h", "\x1b[?2004h",
      "\x1b[?1h", "\x1b=", "\x1b[4h", "\x1b[?6h", "\x1b[?25l", "\x1b[2;5r", "\x1b[1;31m", "\x1b[38;5",
    ].join(""));
    expect(terminal.buffer.active.type).toBe("alternate");
    expect(terminal.modes.mouseTrackingMode).toBe("any");
    expect(terminal.modes.sendFocusMode).toBe(true);

    await write(terminal, RECONNECT_RESET);

    expect(terminal.buffer.active.type).toBe("normal");
    expect(terminal.modes).toMatchObject({
      mouseTrackingMode: "none",
      sendFocusMode: false,
      bracketedPasteMode: false,
      applicationCursorKeysMode: false,
      applicationKeypadMode: false,
      insertMode: false,
      originMode: false,
      wraparoundMode: true,
    });
    expect(coreMouseService.activeProtocol).toBe("NONE");
    expect(coreMouseService.activeEncoding).toBe("DEFAULT");
    // Plain text after the reset is printed as text, in default colors.
    await write(terminal, "$ ");
    const line = terminal.buffer.active.getLine(terminal.buffer.active.cursorY);
    expect(line?.translateToString(true)).toContain("$");
    expect(line?.getCell(terminal.buffer.active.cursorX - 2)?.isAttributeDefault()).toBe(true);
  } finally {
    terminal.dispose();
  }
});
