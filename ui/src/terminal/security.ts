export const MAX_OSC52_BYTES = 64 * 1024;

export type Osc52Request =
  | { kind: "read" }
  | { kind: "write"; text: string }
  | { kind: "invalid" };

export function parseOsc52Request(data: string): Osc52Request {
  const separator = data.indexOf(";");
  if (separator < 0 || separator > 16) return { kind: "invalid" };
  const encoded = data.slice(separator + 1);
  if (encoded === "?") return { kind: "read" };
  if (encoded.length > Math.ceil(MAX_OSC52_BYTES / 3) * 4 + 4) return { kind: "invalid" };
  try {
    const binary = atob(encoded);
    if (binary.length > MAX_OSC52_BYTES) return { kind: "invalid" };
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    return { kind: "write", text: new TextDecoder("utf-8", { fatal: true }).decode(bytes) };
  } catch {
    return { kind: "invalid" };
  }
}

// ESC [ or its one-byte C1 form CSI, then 200~ or 201~.
const PASTE_BRACKET = /(?:\u001b\[|\u009b)20[01]~/gu;
// C0 and C1 controls other than tab and line breaks, plus DEL.
const PASTE_CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/u;

/**
 * Removes bracketed-paste markers. A pasted `ESC[201~` would end the bracket
 * early and the rest would run as typed keys. Repeats until none are left,
 * because removing one marker can join the pieces of another.
 */
export function stripPasteBrackets(text: string): string {
  let current = text;
  for (;;) {
    const next = current.replace(PASTE_BRACKET, "");
    if (next === current) return next;
    current = next;
  }
}

/** Escape, Ctrl key codes and the like act as keystrokes wherever bracketed paste is off. */
export function hasPasteControlCharacters(text: string): boolean {
  return PASTE_CONTROL.test(text);
}

export function isMultilinePaste(text: string): boolean {
  return /[\r\n]/u.test(text);
}

/** Lines the user will see in the confirmation; a final line break does not start another line. */
export function pastedLineCount(text: string): number {
  return text.replace(/(?:\r\n|\r|\n)$/u, "").split(/\r\n|\r|\n/u).length;
}

/**
 * Opens a terminal link, asking first unless it is a web address that reads
 * exactly as it goes. An OSC 8 hyperlink (`hiddenTarget`) shows text the remote
 * chose, and a user name (`http://trusted.test@evil.test`) disguises the host.
 */
export function activateTerminalLink(
  value: string,
  confirm: (url: URL) => boolean | Promise<boolean>,
  open: (url: URL) => void,
  hiddenTarget = false,
): boolean {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return false;
  }
  const web = url.protocol === "http:" || url.protocol === "https:";
  if (web && !hiddenTarget && !url.username && !url.password) {
    open(url);
    return true;
  }
  const answer = confirm(url);
  if (answer instanceof Promise) {
    void answer.then((accepted) => { if (accepted) open(url); }, () => undefined);
    return true;
  }
  if (!answer) return false;
  open(url);
  return true;
}
