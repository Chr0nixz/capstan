import "@xterm/xterm/css/xterm.css";
import { Terminal } from "@xterm/xterm";
import { createTerminalSession } from "../src/terminal/session";
import { activateTerminalLink } from "../src/terminal/security";

const results: { name: string; passed: boolean; errors: string[] }[] = [];
const input: Uint8Array[] = [];
const clipboard: string[] = [];
let allowWrite = false;
let allowPaste = false;
let writePrompts = 0;
let pastePrompts = 0;
let blockedReads = 0;
let parsed: (() => void) | undefined;
const container = document.querySelector<HTMLDivElement>("#terminal");
if (!container) throw new Error("Missing test terminal");
const session = createTerminalSession(container, false, {
  sendInput: async bytes => { input.push(bytes); },
  ackOutput: () => parsed?.(),
  confirmOsc52Write: () => { writePrompts++; return allowWrite; },
  writeOsc52Clipboard: async text => { clipboard.push(text); },
  onOsc52ReadBlocked: () => { blockedReads++; },
  confirmMultilinePaste: () => { pastePrompts++; return allowPaste; },
});
function check(name: string, condition: boolean): void {
  results.push({ name, passed: condition, errors: condition ? [] : ["Policy assertion failed"] });
  if (!condition) throw new Error(name);
}
async function write(value: string): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("xterm parse timeout")), 3000);
    parsed = () => { clearTimeout(timer); parsed = undefined; resolve(); };
    session.write(new TextEncoder().encode(value));
  });
}
function paste(text: string, textarea: HTMLTextAreaElement): ClipboardEvent {
  const data = new DataTransfer();
  data.setData("text/plain", text);
  const event = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true });
  textarea.dispatchEvent(event);
  return event;
}
async function hiddenSelectionRegression(): Promise<void> {
  const host = document.createElement("div");
  host.style.cssText = "position:fixed;top:0;left:0;width:640px;height:160px";
  document.body.appendChild(host);
  const terminal = new Terminal({ cols: 40, rows: 5, scrollback: 1000, allowProposedApi: true });
  const settle = async (): Promise<void> => {
    for (let frame = 0; frame < 3; frame++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  };
  const append = async (data: string): Promise<void> => new Promise<void>(resolve => terminal.write(data, resolve));
  let observer: MutationObserver | undefined;
  try {
    terminal.open(host);
    await append("alpha\r\n");
    await settle();
    host.hidden = true;
    await settle();
    const rows = host.querySelector(".xterm-rows");
    if (!rows) throw new Error("Missing real DOM renderer");
    let mutations = 0;
    observer = new MutationObserver(records => { mutations += records.length; });
    observer.observe(rows, { childList: true, subtree: true, attributes: true });
    await append("alpha\r\n".repeat(60));
    terminal.select(0, terminal.buffer.active.baseY, 5);
    await settle();
    check("hidden-selection-and-scroll-do-not-redraw-DOM-rows", mutations === 0);
    check("hidden-selection-keeps-logical-state", terminal.getSelection() === "alpha");
    host.hidden = false;
    await settle();
    check("visible-again-renders-latest-hidden-selection", !!host.querySelector(".xterm-selection")?.childElementCount);
    host.hidden = true;
    await settle();
    terminal.clearSelection();
    await settle();
    host.hidden = false;
    await settle();
    check("visible-again-applies-hidden-selection-clear", host.querySelector(".xterm-selection")?.childElementCount === 0);
  } finally {
    observer?.disconnect();
    terminal.dispose();
    host.remove();
  }
}
try {
  await write("\x1b]52;c;?\x07");
  check("OSC52-read-denied-without-terminal-reply", blockedReads === 1 && input.length === 0);
  await write("\x1b]52;c;SGVsbG8=\x07");
  check("OSC52-write-requires-consent", writePrompts === 1 && clipboard.length === 0);
  allowWrite = true;
  await write("\x1b]52;c;SGVs");
  check("fragmented-OSC52-waits-for-terminator", writePrompts === 1 && clipboard.length === 0);
  await write("bG8=\x1b\\");
  check("approved-fragmented-OSC52-write", writePrompts === 2 && clipboard.length === 1 && clipboard[0] === "Hello");
  await write("\x1b]52;c;%%%\x07");
  check("malformed-OSC52-does-not-write", writePrompts === 2 && clipboard.length === 1);
  const textarea = container.querySelector<HTMLTextAreaElement>("textarea");
  if (!textarea) throw new Error("Missing real xterm input");
  await write("\x1b[?2004h");
  const denied = paste("first\nsecond", textarea);
  check("multiline-paste-denied-before-input", denied.defaultPrevented && pastePrompts === 1 && input.length === 0);
  allowPaste = true;
  const accepted = paste("first\nsecond", textarea);
  await new Promise(resolve => setTimeout(resolve, 0));
  const bytes = new TextDecoder().decode(input[0]);
  check("approved-paste-keeps-bracketed-paste-markers", accepted.defaultPrevented && pastePrompts === 2 && input.length === 1 && bytes.startsWith("\x1b[200~") && bytes.endsWith("\x1b[201~"));
  let opened = 0;
  check("non-http-link-denied-before-open", !activateTerminalLink("mailto:synthetic@example.test", () => false, () => { opened++; }) && opened === 0);
  check("approved-non-http-link-opens-once", activateTerminalLink("mailto:synthetic@example.test", () => true, () => { opened++; }) && opened === 1);
  session.dispose();
  paste("first\nsecond", textarea);
  check("disposed-paste-listener-does-not-prompt", pastePrompts === 2);
  await hiddenSelectionRegression();
} catch (error) {
  results.push({ name: "harness", passed: false, errors: [error instanceof Error ? error.message : "Unknown test failure"] });
} finally {
  session.dispose();
  const report = {
    passed: results.length === 14 && results.every(result => result.passed),
    limitation: "Real browser/xterm policy integration with an isolated clipboard adapter; no host clipboard reads/writes or native WebView2 acceptance.",
    results,
  };
  const output = document.querySelector<HTMLElement>("#result");
  if (output) output.dataset.report = encodeURIComponent(JSON.stringify(report));
  document.title = report.passed ? "PASS: terminal security" : "FAIL: terminal security";
}
