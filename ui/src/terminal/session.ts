import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";

import { InputQueue } from "./inputQueue";
import {
  findTerminalMatches,
  selectTerminalMatch,
  type SearchTerminal,
  type TerminalSearchOptions,
  type TerminalSearchResult,
} from "./search";
import { isMultilinePaste, parseOsc52Request } from "./security";
import type { TerminalTheme } from "./contracts";
import type { TerminalThemeDefinition } from "../themes/contracts";
import { subscribeTerminalInput } from "../panes/xtermInput";

export type Disposable = { dispose: () => void };

export interface TerminalLike {
  open: (container: HTMLElement) => void;
  loadAddon: (addon: unknown) => void;
  write: (data: Uint8Array, callback?: () => void) => void;
  onData: (listener: (data: string) => void) => Disposable;
  onBinary?: (listener: (data: string) => void) => Disposable;
  onResize: (listener: (size: { cols: number; rows: number }) => void) => Disposable;
  resize: (cols: number, rows: number) => void;
  focus: () => void;
  dispose: () => void;
  unicode?: { activeVersion: string };
  buffer?: {
    active?: {
      length: number;
      get: (row: number) => {
        length: number;
        getCell: (column: number) => { getWidth: () => number; getChars: () => string } | undefined;
      } | undefined;
    };
  };
  clearSelection?: () => void;
  getSelection?: () => string;
  onSelectionChange?: (listener: () => void) => Disposable;
  modes?: { mouseTrackingMode: string };
  scrollToLine?: (line: number) => void;
  select?: (column: number, row: number, length: number) => void;
  textarea?: HTMLTextAreaElement;
  paste?: (text: string) => void;
  parser?: { registerOscHandler: (identifier: number, callback: (data: string) => boolean | Promise<boolean>) => Disposable };
  options?: { theme?: Record<string, string> };
}

/** xterm 5.4 dispatches SGR reports synchronously through onData with the
 * user flag set. Keep their provenance at the mouse service boundary. */
function watchMouseReports(terminal: TerminalLike): (Disposable & { readonly active: boolean }) | undefined {
  const core = "_core" in terminal ? terminal._core : undefined;
  const mouse = typeof core === "object" && core !== null && "coreMouseService" in core ? core.coreMouseService : undefined;
  if (typeof mouse !== "object" || mouse === null || !("triggerMouseEvent" in mouse) || typeof mouse.triggerMouseEvent !== "function") return;

  const original = mouse.triggerMouseEvent as (event: unknown) => boolean;
  let depth = 0;
  const trigger = function (this: typeof mouse, event: unknown): boolean {
    depth += 1;
    try { return original.call(this, event); }
    finally { depth -= 1; }
  };
  try { mouse.triggerMouseEvent = trigger; }
  catch { return; }
  return {
    get active() { return depth > 0; },
    dispose() { if (mouse.triggerMouseEvent === trigger) mouse.triggerMouseEvent = original; },
  };
}

export type RendererAddon = { dispose: () => void; onContextLoss?: (callback: () => void) => Disposable };
export type RendererFactory = (terminal: TerminalLike) => RendererAddon;
export type FitAddonLike = { fit: () => void };

export type TerminalSessionOptions = {
  theme?: TerminalTheme;
  themes?: readonly TerminalThemeDefinition[];
  terminal?: TerminalLike;
  rendererFactory?: RendererFactory;
  fitAddonFactory?: (terminal: TerminalLike) => FitAddonLike;
  inputQueue?: InputQueue;
  sendInput?: (data: Uint8Array) => void | Promise<void>;
  /** Protocol replies and mouse reports bypass user confirmation and remain scoped to this
   * terminal. The receiver owns their bounded destination FIFO. */
  sendProtocolInput?: (data: Uint8Array) => void | Promise<void>;
  ackOutput?: (bytes: number) => void;
  onResize?: (cols: number, rows: number) => void;
  onInputError?: (error: unknown) => void;
  onInputOverflow?: (droppedBytes: number) => void;
  onParsedOutput?: (bytes: Uint8Array) => void;
  confirmMultilinePaste?: (lineCount: number) => boolean;
  copyOnSelect?: () => boolean;
  readClipboard?: () => Promise<string>;
  writeSelectionClipboard?: (text: string) => Promise<void>;
  onSelectionCopyFailed?: () => void;
  onActivateLink?: (uri: string) => void;
  confirmOsc52Write?: (characterCount: number) => boolean;
  writeOsc52Clipboard?: (text: string) => Promise<void>;
  onOsc52ReadBlocked?: () => void;
  onOsc52WriteFailed?: () => void;
  onRemoteCwd?: (uri: string) => void;
};

export type TerminalSession = {
  write: (data: Uint8Array) => void;
  writeLocal: (data: Uint8Array) => void;
  setVisible: (visible: boolean) => void;
  setTheme: (theme: TerminalTheme, themes?: readonly TerminalThemeDefinition[]) => void;
  hasSelection: () => boolean;
  mouseReporting: () => boolean;
  copySelection: () => Promise<void>;
  pasteFromClipboard: () => Promise<void>;
  clearScreen: () => void;
  focus: () => void;
  flushInput: () => Uint8Array[];
  stopInput: () => void;
  resumeInput: () => void;
  settleOutput: () => Promise<void>;
  search: (query: string, options: TerminalSearchOptions, direction: 1 | -1, currentIndex: number) => TerminalSearchResult;
  clearSearch: () => void;
  dispose: () => void;
  get rendererMounted(): boolean;
  get disposed(): boolean;
};

const ansiNames = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite"] as const;

export function terminalPalette(theme: TerminalTheme, themes: readonly TerminalThemeDefinition[] = []): Record<string, string> {
  const imported = themes.find(({ id }) => id === theme);
  if (imported) return {
    background: imported.background,
    foreground: imported.foreground,
    cursor: imported.cursor,
    cursorAccent: imported.background,
    selectionBackground: imported.selectionBackground,
    ...Object.fromEntries(ansiNames.map((name, index) => [name, imported.ansi[index]])),
  };
  return theme === "light" ? {
    background: "#f7f8fa",
    foreground: "#24292f",
    cursor: "#24292f",
    cursorAccent: "#f7f8fa",
    selectionBackground: "#b6d7ff",
    black: "#24292f",
    red: "#cf222e",
    green: "#116329",
    yellow: "#4d2d00",
    blue: "#0969da",
    magenta: "#8250df",
    cyan: "#1b7c83",
    white: "#6e7781",
    brightBlack: "#57606a",
    brightRed: "#a40e26",
    brightGreen: "#1a7f37",
    brightYellow: "#633c01",
    brightBlue: "#218bff",
    brightMagenta: "#a475f9",
    brightCyan: "#3192aa",
    brightWhite: "#8c959f",
  } : {
    background: "#16171a",
    foreground: "#e4e0d7",
    cursor: "#d9a441",
    cursorAccent: "#16171a",
    selectionBackground: "#4a3f28",
    black: "#2b2e33",
    red: "#ff7b72",
    green: "#7ee787",
    yellow: "#d29922",
    blue: "#79c0ff",
    magenta: "#d2a8ff",
    cyan: "#a5d6ff",
    white: "#cdc9c0",
    brightBlack: "#6e7681",
    brightRed: "#ffa198",
    brightGreen: "#56d364",
    brightYellow: "#e3b341",
    brightBlue: "#a5d6ff",
    brightMagenta: "#a475f9",
    brightCyan: "#a5d6ff",
    brightWhite: "#f4f1ea",
  };
}

function defaultTerminal({ theme = "graphite", themes, onActivateLink }: TerminalSessionOptions): TerminalLike {
  const terminal = new Terminal({
    fontFamily: '"Cascadia Mono", "Microsoft YaHei UI", Consolas, monospace',
    allowTransparency: false,
    allowProposedApi: true,
    theme: terminalPalette(theme, themes),
    linkHandler: onActivateLink ? { activate: (_event, uri) => onActivateLink(uri) } : undefined,
  });
  return terminal as unknown as TerminalLike;
}

function defaultRendererFactory(terminal: TerminalLike): RendererAddon {
  const addon = new WebglAddon();
  try { (terminal as unknown as Terminal).loadAddon(addon); }
  catch (error) { addon.dispose(); throw error; }
  return addon;
}

/** Owns xterm listeners and renderer resources for one visible terminal. */
export function createTerminalSession(
  container: HTMLElement,
  visible: boolean,
  options: TerminalSessionOptions = {},
): TerminalSession {
  const terminal = options.terminal ?? defaultTerminal(options);
  const fit = options.fitAddonFactory?.(terminal) ?? (() => {
    const addon = new FitAddon();
    terminal.loadAddon(addon);
    return addon;
  })();
  const unicode = new Unicode11Addon();
  // FitAddon sizes the grid from the parent's computed size and subtracts only
  // the .xterm element's own padding. A bare mount keeps the caller's padding
  // and borders out of that measurement so the last row is never clipped.
  const mount = container.ownerDocument.createElement("div");
  mount.style.width = "100%";
  mount.style.height = "100%";
  container.appendChild(mount);
  terminal.open(mount);
  terminal.loadAddon(unicode);
  if (terminal.unicode) {
    terminal.unicode.activeVersion = "11";
  }

  const queue = options.inputQueue ?? new InputQueue();
  const sendInput = options.sendInput;
  let renderer: RendererAddon | undefined;
  let contextLoss: Disposable | undefined;
  let isDisposed = false;
  let isVisible = visible;
  let sendingInput = false;

  const fitTerminal = (): void => {
    if (!isDisposed && isVisible) {
      try {
        fit.fit();
      } catch {
        // The container may not have measurable dimensions during initial mount.
      }
    }
  };

  let inputEpoch = 0;
  const pumpInput = async (): Promise<void> => {
    if (isDisposed || sendingInput || !sendInput) {
      return;
    }
    sendingInput = true;
    const epoch = inputEpoch;
    try {
      while (!isDisposed && epoch === inputEpoch) {
        const next = queue.peek();
        if (!next) {
          break;
        }
        try {
          await sendInput(next);
          if (epoch !== inputEpoch) break;
          queue.dequeue();
        } catch (error) {
          if (epoch !== inputEpoch) break;
          // The backend may have consumed part of a failed write. Replaying
          // either that request or later queued keystrokes is unsafe.
          queue.close();
          options.onInputError?.(error);
          break;
        }
      }
    } finally {
      if (epoch === inputEpoch) sendingInput = false;
    }
  };

  const resizeObserver = typeof ResizeObserver === "undefined"
    ? undefined
    : new ResizeObserver(fitTerminal);
  resizeObserver?.observe(container);

  const enqueueInput = (bytes: Uint8Array): void => {
    const result = queue.enqueue(bytes);
    if (result.accepted) void pumpInput();
    else options.onInputOverflow?.(result.droppedBytes);
  };
  const sendProtocolInput = (bytes: Uint8Array): void => {
    if (!options.sendProtocolInput) {
      enqueueInput(bytes);
      return;
    }
    const failed = (error: unknown): void => {
      if (isDisposed || queue.isClosed) return;
      queue.close();
      options.onInputError?.(error);
    };
    try { void Promise.resolve(options.sendProtocolInput(bytes)).catch(failed); }
    catch (error) { failed(error); }
  };
  const mouseReporting = (): boolean => (terminal.modes?.mouseTrackingMode ?? "none") !== "none";
  const mouseReports = watchMouseReports(terminal);
  const dataDisposable = subscribeTerminalInput(terminal, (data, origin) => {
    if (isDisposed || queue.isClosed) {
      return;
    }
    const mouseReport = mouseReports?.active ?? false;
    if (mouseReport && !isVisible) return;
    if (options.sendProtocolInput && !mouseReport
      && (origin === "unknown" || (origin === "user" && !mouseReports && mouseReporting()))) {
      // Managed panes cannot safely route unclassified input after an xterm
      // API change. Stop instead of guessing a broadcast or local destination.
      queue.close();
      options.onInputError?.({ code: "terminal_input_origin_unknown" });
      return;
    }
    const bytes = new TextEncoder().encode(data);
    if (mouseReport || origin === "protocol") sendProtocolInput(bytes);
    else enqueueInput(bytes);
  });
  const binaryDisposable = terminal.onBinary?.((data) => {
    if (isDisposed || queue.isClosed || !isVisible) return;
    // xterm uses its binary event for legacy mouse encodings. Coordinates
    // belong to the source pane and the payload is already one byte per code unit.
    sendProtocolInput(Uint8Array.from(data, (character) => character.charCodeAt(0)));
  });
  const resizeDisposable = terminal.onResize(({ cols, rows }) => {
    if (!isDisposed) {
      options.onResize?.(cols, rows);
    }
  });
  const pasteText = (text: string): void => {
    if (text && (!isMultilinePaste(text) || options.confirmMultilinePaste?.(text.split(/\r\n|\r|\n/u).length))) terminal.paste?.(text);
  };
  const pasteListener = (event: ClipboardEvent): void => {
    const text = event.clipboardData?.getData("text/plain");
    if (text === undefined || !isMultilinePaste(text)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    pasteText(text);
  };
  terminal.textarea?.addEventListener("paste", pasteListener, true);
  let selectingSearchResult = false;
  const copySelection = async (): Promise<void> => {
    if (isDisposed || !isVisible) return;
    const text = terminal.getSelection?.();
    if (text) await options.writeSelectionClipboard?.(text);
  };
  const selectionDisposable = terminal.onSelectionChange?.(() => {
    if (!selectingSearchResult && options.copyOnSelect?.()) {
      void copySelection().catch(() => options.onSelectionCopyFailed?.());
    }
  });
  const osc52Disposable = terminal.parser?.registerOscHandler(52, (data) => {
    const request = parseOsc52Request(data);
    if (request.kind === "read") {
      options.onOsc52ReadBlocked?.();
      return true;
    }
    if (request.kind === "write") {
      if (!options.confirmOsc52Write?.(request.text.length)) return true;
      void options.writeOsc52Clipboard?.(request.text).catch(() => options.onOsc52WriteFailed?.());
    }
    return true;
  });
  const osc7Disposable = terminal.parser?.registerOscHandler(7, (data) => {
    if (!isDisposed) options.onRemoteCwd?.(data);
    return true;
  });

  const mountRenderer = (): void => {
    if (isDisposed || renderer) {
      return;
    }
    try {
      renderer = (options.rendererFactory ?? defaultRendererFactory)(terminal);
      contextLoss = renderer.onContextLoss?.(() => unmountRenderer());
    } catch {
      // xterm's default renderer remains active when WebGL is unavailable.
      renderer = undefined;
    }
  };

  const unmountRenderer = (): void => {
    contextLoss?.dispose();
    contextLoss = undefined;
    renderer?.dispose();
    renderer = undefined;
  };

  const write = (data: Uint8Array, acknowledge: boolean): void => {
    if (isDisposed) {
      return;
    }
    terminal.write(data, acknowledge ? () => {
      if (!isDisposed) {
        options.onParsedOutput?.(data);
        options.ackOutput?.(data.byteLength);
      }
    } : undefined);
  };

  if (visible) {
    fitTerminal();
    mountRenderer();
  }

  return {
    write(data: Uint8Array): void {
      // ACK uses the exact byte count supplied to xterm, rather than the
      // number of messages delivered to the renderer.
      write(data, true);
    },
    writeLocal(data: Uint8Array): void {
      write(data, false);
    },
    settleOutput(): Promise<void> {
      if (isDisposed) return Promise.resolve();
      return new Promise((resolve) => terminal.write(new Uint8Array(), resolve));
    },
    setVisible(nextVisible: boolean): void {
      if (isDisposed) {
        return;
      }
      if (nextVisible) {
        isVisible = true;
        fitTerminal();
        mountRenderer();
      } else {
        isVisible = false;
        unmountRenderer();
      }
    },
    setTheme(nextTheme: TerminalTheme, themes): void {
      if (isDisposed || !terminal.options) return;
      terminal.options.theme = terminalPalette(nextTheme, themes);
    },
    hasSelection: () => !!terminal.getSelection?.(),
    mouseReporting,
    copySelection,
    async pasteFromClipboard(): Promise<void> {
      const epoch = inputEpoch;
      const text = await options.readClipboard?.();
      // Clipboard reads are asynchronous. Keep a paste scoped to its pane.
      if (text && !isDisposed && isVisible && !queue.isClosed && epoch === inputEpoch
        && (!terminal.textarea || terminal.textarea === document.activeElement)) pasteText(text);
    },
    clearScreen(): void {
      // ED 2 and ED 3 clear the viewport and scrollback in xterm locally.
      write(new TextEncoder().encode("\x1b[2J\x1b[3J\x1b[H"), false);
    },
    focus(): void {
      if (!isDisposed && isVisible) {
        terminal.focus();
      }
    },
    flushInput(): Uint8Array[] {
      return queue.drain();
    },
    stopInput(): void { inputEpoch += 1; sendingInput = false; queue.close(); },
    resumeInput(): void { inputEpoch += 1; sendingInput = false; queue.reopen(); },
    search(query, searchOptions, direction, currentIndex): TerminalSearchResult {
      const found = findTerminalMatches(terminal.buffer?.active, query, searchOptions);
      if (found.invalidExpression) {
        terminal.clearSelection?.();
        return { index: -1, count: 0, match: null, invalidExpression: true };
      }
      if (!terminal.clearSelection || !terminal.scrollToLine || !terminal.select) {
        return { index: -1, count: 0, match: null, invalidExpression: false };
      }
      const searchTerminal: SearchTerminal = {
        buffer: { active: terminal.buffer?.active },
        clearSelection: () => terminal.clearSelection?.(),
        scrollToLine: (line) => terminal.scrollToLine?.(line),
        select: (column, row, length) => terminal.select?.(column, row, length),
      };
      selectingSearchResult = true;
      try { return selectTerminalMatch(searchTerminal, found.matches, currentIndex, direction, query); }
      finally { selectingSearchResult = false; }
    },
    clearSearch(): void { terminal.clearSelection?.(); },
    dispose(): void {
      if (isDisposed) {
        return;
      }
      isDisposed = true;
      dataDisposable.dispose();
      binaryDisposable?.dispose();
      mouseReports?.dispose();
      resizeDisposable.dispose();
      selectionDisposable?.dispose();
      terminal.textarea?.removeEventListener("paste", pasteListener, true);
      osc52Disposable?.dispose();
      osc7Disposable?.dispose();
      resizeObserver?.disconnect();
      unmountRenderer();
      queue.close();
      mount.remove();
      // xterm 5.4 schedules a viewport sync with setTimeout when it opens;
      // disposing first (a quick close, or a StrictMode remount) makes that
      // timer read a released renderer. Queue the disposal behind it.
      setTimeout(() => terminal.dispose(), 0);
    },
    get rendererMounted(): boolean {
      return renderer !== undefined;
    },
    get disposed(): boolean {
      return isDisposed;
    },
  };
}
