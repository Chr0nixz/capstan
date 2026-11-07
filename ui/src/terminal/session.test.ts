import { describe, expect, it, vi } from "vitest";

import { createTerminalSession, terminalPalette, type Disposable, type TerminalLike } from "./session";
import { InputQueue } from "./inputQueue";
import fixture from "../../../docs/agent/fixtures/M4-contract.json";
import { isTerminalTheme, type TerminalThemeDefinition } from "../themes/contracts";

const defaultTerminalMock = vi.hoisted(() => ({
  options: undefined as Record<string, unknown> | undefined,
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    unicode = { activeVersion: "6" };

    constructor(options: Record<string, unknown>) {
      defaultTerminalMock.options = options;
    }

    open(_container: HTMLElement): void {}

    loadAddon(_addon: unknown): void {}

    write(_data: Uint8Array, callback?: () => void): void {
      callback?.();
    }

    onData(_listener: (data: string) => void): Disposable {
      return { dispose: () => undefined };
    }

    onResize(_listener: (size: { cols: number; rows: number }) => void): Disposable {
      return { dispose: () => undefined };
    }

    resize(_cols: number, _rows: number): void {}

    dispose(): void {}
  },
}));

function fakeTerminal(): TerminalLike & {
  dataListener?: (data: string) => void;
  resizeListener?: (size: { cols: number; rows: number }) => void;
  oscHandler?: (data: string) => boolean | Promise<boolean>;
  disposeCount: number;
} {
  let terminal!: TerminalLike & {
    dataListener?: (data: string) => void;
    resizeListener?: (size: { cols: number; rows: number }) => void;
    oscHandler?: (data: string) => boolean | Promise<boolean>;
    disposeCount: number;
  };
  terminal = {
    disposeCount: 0,
    open: vi.fn(),
    loadAddon: vi.fn(),
    write: vi.fn((_: Uint8Array, callback?: () => void) => callback?.()),
    onData(listener: (data: string) => void): Disposable {
      terminal.dataListener = listener;
      return { dispose: vi.fn() };
    },
    onResize(listener: (size: { cols: number; rows: number }) => void): Disposable {
      terminal.resizeListener = listener;
      return { dispose: vi.fn() };
    },
    resize: vi.fn(),
    focus: vi.fn(),
    textarea: document.createElement("textarea"),
    paste: vi.fn(),
    parser: {
      registerOscHandler: vi.fn((_identifier: number, handler: (data: string) => boolean | Promise<boolean>) => {
        if (_identifier === 52) terminal.oscHandler = handler;
        return { dispose: vi.fn() };
      }),
    },
    dispose: vi.fn(() => {
      terminal.disposeCount += 1;
    }),
  };
  return terminal;
}

describe("terminal session lifecycle", () => {
  it("maps a custom palette and falls back after its definition is removed", () => {
    const theme = fixture.terminalTheme as TerminalThemeDefinition;
    expect(isTerminalTheme(theme.id)).toBe(true);
    expect(terminalPalette(theme.id, [theme])).toMatchObject({ background: theme.background, foreground: theme.foreground,
      cursor: theme.cursor, selectionBackground: theme.selectionBackground, black: theme.ansi[0], brightWhite: theme.ansi[15] });
    expect(terminalPalette(theme.id)).toEqual(terminalPalette("graphite"));
  });

  it("copies selections only after opt-in and clears through local output without acknowledging remote bytes", async () => {
    const terminal = fakeTerminal();
    let selectionChanged: (() => void) | undefined;
    let enabled = false;
    terminal.getSelection = () => "selected";
    terminal.onSelectionChange = (listener) => { selectionChanged = listener; return { dispose: vi.fn() }; };
    const writeSelectionClipboard = vi.fn(async () => undefined);
    const readClipboard = vi.fn(async () => "paste");
    const sendInput = vi.fn(), ackOutput = vi.fn();
    const session = createTerminalSession(document.createElement("div"), true, { terminal, sendInput, ackOutput,
      writeSelectionClipboard, readClipboard, copyOnSelect: () => enabled, rendererFactory: () => ({ dispose: vi.fn() }) });
    selectionChanged?.();
    expect(writeSelectionClipboard).not.toHaveBeenCalled();
    enabled = true;
    selectionChanged?.();
    expect(writeSelectionClipboard).toHaveBeenCalledWith("selected");
    session.clearScreen();
    expect(terminal.write).toHaveBeenCalledWith(new TextEncoder().encode("\x1b[2J\x1b[3J\x1b[H"), undefined);
    expect(sendInput).not.toHaveBeenCalled();
    expect(ackOutput).not.toHaveBeenCalled();
    expect(readClipboard).not.toHaveBeenCalled();
    await session.copySelection();
    expect(writeSelectionClipboard).toHaveBeenCalledTimes(2);
    session.dispose();
  });

  it("gates menu paste through multiline confirmation and drops results after focus or shell changes", async () => {
    const terminal = fakeTerminal();
    document.body.append(terminal.textarea!);
    terminal.textarea!.focus();
    const confirmMultilinePaste = vi.fn(() => false);
    const readClipboard = vi.fn(async () => "first\nsecond");
    const session = createTerminalSession(document.createElement("div"), true, { terminal, confirmMultilinePaste, readClipboard,
      rendererFactory: () => ({ dispose: vi.fn() }) });
    await session.pasteFromClipboard();
    expect(confirmMultilinePaste).toHaveBeenCalledWith(2);
    expect(terminal.paste).not.toHaveBeenCalled();
    confirmMultilinePaste.mockReturnValue(true);
    await session.pasteFromClipboard();
    expect(terminal.paste).toHaveBeenCalledWith("first\nsecond");
    const pending = session.pasteFromClipboard();
    terminal.textarea!.blur();
    await pending;
    expect(terminal.paste).toHaveBeenCalledTimes(1);
    terminal.textarea!.focus();
    const pendingReconnect = session.pasteFromClipboard();
    session.stopInput();
    session.resumeInput();
    await pendingReconnect;
    expect(terminal.paste).toHaveBeenCalledTimes(1);
    session.dispose();
    terminal.textarea!.remove();
  });

  it("keeps legacy xterm mouse reports binary and scoped to the source pane", () => {
    const terminal = fakeTerminal();
    let report: ((data: string) => void) | undefined;
    terminal.onBinary = (listener) => { report = listener; return { dispose: vi.fn() }; };
    const sendInput = vi.fn(), sendProtocolInput = vi.fn();
    const session = createTerminalSession(document.createElement("div"), true, { terminal, sendInput, sendProtocolInput,
      rendererFactory: () => ({ dispose: vi.fn() }) });
    report?.("\x1b[M\xa3");
    expect(sendProtocolInput).toHaveBeenCalledWith(new Uint8Array([27, 91, 77, 163]));
    expect(sendInput).not.toHaveBeenCalled();
    session.setVisible(false);
    report?.("\x1b[M\xa3");
    expect(sendProtocolInput).toHaveBeenCalledTimes(1);
    session.dispose();
  });
  it("drops queued old input and ignores a late rejected write after resuming a new shell", async () => {
    const terminal = fakeTerminal();
    let rejectOld!: (error: unknown) => void;
    const oldWrite = new Promise<void>((_resolve, reject) => { rejectOld = reject; });
    const sendInput = vi.fn<(bytes: Uint8Array) => Promise<void>>().mockImplementationOnce(() => oldWrite).mockResolvedValue(undefined);
    const inputError = vi.fn();
    const session = createTerminalSession(document.createElement("div"), false, { terminal, sendInput, onInputError: inputError });
    terminal.dataListener?.("old-sent");
    terminal.dataListener?.("old-queued");
    session.stopInput();
    terminal.dataListener?.("while-disconnected");
    session.resumeInput();
    terminal.dataListener?.("new-input");
    rejectOld(new Error("old write outcome unknown"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    terminal.dataListener?.("next-input");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(sendInput.mock.calls.map(([bytes]) => new TextDecoder().decode(bytes))).toEqual(["old-sent", "new-input", "next-input"]);
    expect(inputError).not.toHaveBeenCalled();
    session.dispose();
  });
  it("sends protocol replies while a user confirmation is pending without mixing either route", async () => {
    const terminal = fakeTerminal(), queue = new InputQueue();
    const core = { triggerDataEvent: (data: string, _user = false): void => { terminal.dataListener?.(data); } };
    Object.assign(terminal, { _core: { coreService: core } });
    let release!: () => void;
    const userGate = new Promise<void>((resolve) => { release = resolve; });
    const sendInput = vi.fn(() => userGate), sendProtocolInput = vi.fn(async () => undefined);
    const session = createTerminalSession(document.createElement("div"), false, { terminal, inputQueue: queue, sendInput, sendProtocolInput });
    core.triggerDataEvent("user\r", true);
    core.triggerDataEvent("\x1b[1;1R");
    core.triggerDataEvent("\x1b[1;1R", true);
    expect(sendInput).toHaveBeenCalledExactlyOnceWith(new TextEncoder().encode("user\r"));
    expect(sendProtocolInput).toHaveBeenCalledExactlyOnceWith(new TextEncoder().encode("\x1b[1;1R"));
    release(); await Promise.resolve(); await Promise.resolve();
    expect(sendInput).toHaveBeenCalledTimes(2);
    expect(sendInput).toHaveBeenLastCalledWith(new TextEncoder().encode("\x1b[1;1R"));
    session.dispose();
  });

  it("stops unclassified managed input without sending or exposing its contents", () => {
    const terminal = fakeTerminal(), queue = new InputQueue();
    const sendInput = vi.fn(), sendProtocolInput = vi.fn(), onInputError = vi.fn();
    const session = createTerminalSession(document.createElement("div"), false, { terminal, inputQueue: queue, sendInput, sendProtocolInput, onInputError });
    terminal.dataListener?.("private input");
    terminal.dataListener?.("later input");
    expect(sendInput).not.toHaveBeenCalled(); expect(sendProtocolInput).not.toHaveBeenCalled();
    expect(onInputError).toHaveBeenCalledExactlyOnceWith({ code: "terminal_input_origin_unknown" });
    expect(queue.isClosed).toBe(true);
    session.dispose();
  });

  it("fails closed if xterm mouse provenance is unavailable in a managed pane", () => {
    const terminal = fakeTerminal();
    terminal.modes = { mouseTrackingMode: "vt200" };
    const coreService = { triggerDataEvent: (data: string, _user = false): void => { terminal.dataListener?.(data); } };
    const coreMouseService = Object.freeze({ triggerMouseEvent: () => true });
    Object.assign(terminal, { _core: { coreService, coreMouseService } });
    const sendInput = vi.fn(), sendProtocolInput = vi.fn(), onInputError = vi.fn();
    const session = createTerminalSession(document.createElement("div"), false, { terminal, sendInput, sendProtocolInput, onInputError });
    try {
      coreService.triggerDataEvent("\x1b[<2;1;1M", true);
      expect(sendInput).not.toHaveBeenCalled();
      expect(sendProtocolInput).not.toHaveBeenCalled();
      expect(onInputError).toHaveBeenCalledExactlyOnceWith({ code: "terminal_input_origin_unknown" });
    } finally { session.dispose(); }
  });

  it("closes input once after a protocol write fails and never replays it", async () => {
    const terminal = fakeTerminal(), queue = new InputQueue();
    const core = { triggerDataEvent: (data: string, _user = false): void => { terminal.dataListener?.(data); } };
    Object.assign(terminal, { _core: { coreService: core } });
    const sendInput = vi.fn(), sendProtocolInput = vi.fn(async () => { throw { code: "delivery_unknown" }; }), onInputError = vi.fn();
    const session = createTerminalSession(document.createElement("div"), false, { terminal, inputQueue: queue, sendInput, sendProtocolInput, onInputError });
    core.triggerDataEvent("\x1b[1;1R");
    await Promise.resolve(); await Promise.resolve();
    core.triggerDataEvent("\x1b[1;1R");
    expect(sendInput).not.toHaveBeenCalled(); expect(sendProtocolInput).toHaveBeenCalledTimes(1);
    expect(onInputError).toHaveBeenCalledExactlyOnceWith({ code: "delivery_unknown" });
    expect(queue.isClosed).toBe(true);
    session.dispose();
  });

  it("forwards only parsed OSC 7 hints synchronously and disposes its parser subscription", () => {
    const terminal = fakeTerminal();
    const onRemoteCwd = vi.fn();
    const handlers = new Map<number, (data: string) => boolean | Promise<boolean>>();
    const dispose = vi.fn();
    terminal.parser = { registerOscHandler: (identifier, handler) => { handlers.set(identifier, handler); return { dispose }; } };
    const session = createTerminalSession(document.createElement("div"), false, { terminal, onRemoteCwd });
    expect(handlers.get(7)?.("file://host/%E4%B8%AD%20dir")).toBe(true);
    expect(onRemoteCwd).toHaveBeenCalledExactlyOnceWith("file://host/%E4%B8%AD%20dir");
    session.dispose();
    expect(dispose).toHaveBeenCalledTimes(2);
    handlers.get(7)?.("file://host/late");
    expect(onRemoteCwd).toHaveBeenCalledTimes(1);
  });
  it("opens xterm in a bare child so the container's padding is not measured by fit", () => {
    const terminal = fakeTerminal();
    const container = document.createElement("div");
    container.style.padding = "10px";
    container.style.border = "1px solid";
    const session = createTerminalSession(container, false, { terminal });
    const mount = container.firstElementChild as HTMLElement;
    expect(terminal.open).toHaveBeenCalledWith(mount);
    expect(mount.style.cssText).toBe("width: 100%; height: 100%;");
    session.dispose();
    expect(container.childElementCount).toBe(0);
  });
  it("takes focus only while visible", () => {
    const terminal = fakeTerminal();
    const session = createTerminalSession(document.createElement("div"), true, { terminal, rendererFactory: () => ({ dispose: vi.fn() }) });
    session.focus();
    expect(terminal.focus).toHaveBeenCalledTimes(1);
    session.setVisible(false);
    session.focus();
    session.dispose();
    session.focus();
    expect(terminal.focus).toHaveBeenCalledTimes(1);
  });
  it("releases a lost WebGL context and can mount a replacement", () => {
    const terminal = fakeTerminal(); let loseContext!: () => void;
    const contextListener = { dispose: vi.fn() };
    const renderer = { dispose: vi.fn(), onContextLoss: (callback: () => void) => { loseContext = callback; return contextListener; } };
    const session = createTerminalSession(document.createElement("div"), true, { terminal, rendererFactory: () => renderer });
    loseContext();
    expect(session.rendererMounted).toBe(false);
    expect(renderer.dispose).toHaveBeenCalledTimes(1);
    expect(contextListener.dispose).toHaveBeenCalledTimes(1);
    session.setVisible(true);
    expect(session.rendererMounted).toBe(true);
    session.dispose();
    expect(renderer.dispose).toHaveBeenCalledTimes(2);
  });
  it("discards uncertain input and never replays it on later keystrokes", async () => {
    const terminal = fakeTerminal();
    const queue = new InputQueue();
    let rejectWrite!: (error: Error) => void;
    const sendInput = vi.fn(() => new Promise<void>((_resolve, reject) => { rejectWrite = reject; }));
    const onInputError = vi.fn();
    const session = createTerminalSession(document.createElement("div"), false, { terminal, inputQueue: queue, sendInput, onInputError });
    terminal.dataListener?.("a");
    terminal.dataListener?.("b");
    rejectWrite(new Error("delivery uncertain"));
    await Promise.resolve();
    await Promise.resolve();
    terminal.dataListener?.("c");
    expect(sendInput).toHaveBeenCalledTimes(1);
    expect(onInputError).toHaveBeenCalledTimes(1);
    expect(queue.isClosed).toBe(true);
    expect(queue.bytes).toBe(0);
    session.dispose();
  });
  it("enables the proposed xterm API required by Unicode 11", () => {
    const session = createTerminalSession(document.createElement("div"), false, {
      fitAddonFactory: () => ({ fit: () => undefined }),
    });

    expect(defaultTerminalMock.options?.allowProposedApi).toBe(true);
    session.dispose();
  });

  it("passes the selected terminal theme to xterm", () => {
    const session = createTerminalSession(document.createElement("div"), false, {
      theme: "light",
      fitAddonFactory: () => ({ fit: () => undefined }),
    });

    expect(defaultTerminalMock.options?.theme).toEqual(expect.objectContaining({
      background: "#f7f8fa",
      foreground: "#24292f",
    }));
    session.dispose();
  });

  it("changes the xterm palette in place without disposing the session", () => {
    const terminal = fakeTerminal();
    terminal.options = { theme: {} };
    const session = createTerminalSession(document.createElement("div"), false, { terminal, theme: "graphite" });
    session.setTheme("light");
    expect(terminal.options.theme?.background).toBe("#f7f8fa");
    expect(terminal.disposeCount).toBe(0);
    session.dispose();
  });

  it("mounts WebGL only while visible and disposes it once", () => {
    const terminal = fakeTerminal();
    const renderer = { dispose: vi.fn() };
    const session = createTerminalSession(document.createElement("div"), true, {
      terminal,
      rendererFactory: vi.fn(() => renderer),
    });
    expect(session.rendererMounted).toBe(true);
    session.setVisible(false);
    expect(renderer.dispose).toHaveBeenCalledTimes(1);
    session.setVisible(true);
    expect(session.rendererMounted).toBe(true);
    vi.useFakeTimers();
    try {
      session.dispose();
      session.dispose();
      expect(renderer.dispose).toHaveBeenCalledTimes(2);
      // xterm itself is released after its pending open-time viewport sync.
      expect(terminal.disposeCount).toBe(0);
      vi.runAllTimers();
      expect(terminal.disposeCount).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("writes binary data and registers input listeners once", () => {
    const terminal = fakeTerminal();
    const sendInput = vi.fn();
    const ackOutput = vi.fn();
    const session = createTerminalSession(document.createElement("div"), false, {
      terminal,
      sendInput,
      ackOutput,
    });
    session.write(new Uint8Array([0, 255, 1]));
    expect(terminal.write).toHaveBeenCalledWith(expect.any(Uint8Array), expect.any(Function));
    expect(ackOutput).toHaveBeenCalledWith(3);
    terminal.dataListener?.("中");
    expect(sendInput).toHaveBeenCalledTimes(1);
    session.dispose();
    expect(session.disposed).toBe(true);
  });

  it("confirms multiline paste and blocks OSC 52 clipboard reads by default", async () => {
    const terminal = fakeTerminal();
    let pasteAllowed = false;
    const confirmPaste = vi.fn(() => pasteAllowed);
    const confirmOsc52 = vi.fn(() => false);
    const writeClipboard = vi.fn(async () => undefined);
    const blockedRead = vi.fn();
    const session = createTerminalSession(document.createElement("div"), false, {
      terminal,
      confirmMultilinePaste: confirmPaste,
      confirmOsc52Write: confirmOsc52,
      writeOsc52Clipboard: writeClipboard,
      onOsc52ReadBlocked: blockedRead,
    });
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", { value: { getData: () => "first\nsecond" } });
    terminal.textarea?.dispatchEvent(paste);
    expect(confirmPaste).toHaveBeenCalledWith(2);
    expect(paste.defaultPrevented).toBe(true);
    expect(terminal.paste).not.toHaveBeenCalled();
    pasteAllowed = true;
    const approvedPaste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(approvedPaste, "clipboardData", { value: { getData: () => "first\nsecond" } });
    terminal.textarea?.dispatchEvent(approvedPaste);
    expect(terminal.paste).toHaveBeenCalledWith("first\nsecond");

    expect(await terminal.oscHandler?.("c;?")).toBe(true);
    expect(blockedRead).toHaveBeenCalledTimes(1);
    expect(await terminal.oscHandler?.("c;SGVsbG8=" )).toBe(true);
    expect(confirmOsc52).toHaveBeenCalledWith(5);
    expect(writeClipboard).not.toHaveBeenCalled();
    session.dispose();
  });

  it("keeps input queued until the asynchronous write completes and fits visible terminals", async () => {
    const terminal = fakeTerminal();
    terminal.unicode = { activeVersion: "6" };
    const queue = new InputQueue({ maxBytes: 4, maxItems: 8 });
    const fit = { fit: vi.fn() };
    let resolveWrite!: () => void;
    const sendInput = vi.fn(() => new Promise<void>((resolve) => {
      resolveWrite = resolve;
    }));
    const session = createTerminalSession(document.createElement("div"), true, {
      terminal,
      inputQueue: queue,
      fitAddonFactory: () => fit,
      sendInput,
    });

    expect(fit.fit).toHaveBeenCalledTimes(1);
    expect(terminal.unicode.activeVersion).toBe("11");
    terminal.dataListener?.("a");
    expect(queue.bytes).toBe(1);
    terminal.dataListener?.("b");
    terminal.dataListener?.("c");
    terminal.dataListener?.("d");
    terminal.dataListener?.("e");
    expect(queue.bytes).toBe(4);
    expect(sendInput).toHaveBeenCalledTimes(1);

    resolveWrite();
    await Promise.resolve();
    await Promise.resolve();
    expect(queue.bytes).toBe(3);
    session.setVisible(false);
    session.setVisible(true);
    expect(fit.fit).toHaveBeenCalledTimes(2);
    session.dispose();
  });
});
