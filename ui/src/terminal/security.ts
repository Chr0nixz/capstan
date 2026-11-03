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

export function isMultilinePaste(text: string): boolean {
  return /[\r\n]/u.test(text);
}

export function activateTerminalLink(
  value: string,
  confirmNonHttp: (url: URL) => boolean,
  open: (url: URL) => void,
): boolean {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return false;
  }
  if (url.protocol === "http:" || url.protocol === "https:") {
    open(url);
    return true;
  }
  if (!confirmNonHttp(url)) return false;
  open(url);
  return true;
}
