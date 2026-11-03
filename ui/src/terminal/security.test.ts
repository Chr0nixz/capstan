import { describe, expect, it, vi } from "vitest";
import { activateTerminalLink, isMultilinePaste, parseOsc52Request } from "./security";

describe("terminal interaction security", () => {
  it("blocks clipboard reads and accepts only bounded UTF-8 OSC 52 writes", () => {
    expect(parseOsc52Request("c;?")).toEqual({ kind: "read" });
    expect(parseOsc52Request("c;SGVsbG8=" )).toEqual({ kind: "write", text: "Hello" });
    expect(parseOsc52Request("c;%%%" )).toEqual({ kind: "invalid" });
  });

  it("detects multi-line paste and gates non-HTTP links separately", () => {
    expect(isMultilinePaste("single line")).toBe(false);
    expect(isMultilinePaste("first\nsecond")).toBe(true);
    const open = vi.fn();
    expect(activateTerminalLink("https://example.test/path", () => false, open)).toBe(true);
    expect(activateTerminalLink("mailto:hello@example.test", () => false, open)).toBe(false);
    expect(activateTerminalLink("mailto:hello@example.test", () => true, open)).toBe(true);
    expect(open).toHaveBeenCalledTimes(2);
  });
});
