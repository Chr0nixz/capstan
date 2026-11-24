import { describe, expect, it, vi } from "vitest";
import { activateTerminalLink, isMultilinePaste, parseOsc52Request, pastedLineCount } from "./security";

describe("terminal interaction security", () => {
  it("blocks clipboard reads and accepts only bounded UTF-8 OSC 52 writes", () => {
    expect(parseOsc52Request("c;?")).toEqual({ kind: "read" });
    expect(parseOsc52Request("c;SGVsbG8=" )).toEqual({ kind: "write", text: "Hello" });
    expect(parseOsc52Request("c;%%%" )).toEqual({ kind: "invalid" });
  });

  it("detects multi-line paste and gates non-HTTP links separately", async () => {
    expect(isMultilinePaste("single line")).toBe(false);
    expect(isMultilinePaste("first\nsecond")).toBe(true);
    expect(isMultilinePaste("ls\n")).toBe(true);
    expect(pastedLineCount("ls\n")).toBe(1);
    expect(pastedLineCount("first\r\nsecond\r\n")).toBe(2);
    expect(pastedLineCount("first\n\nthird")).toBe(3);
    const open = vi.fn();
    expect(activateTerminalLink("https://example.test/path", () => false, open)).toBe(true);
    expect(activateTerminalLink("mailto:hello@example.test", () => false, open)).toBe(false);
    expect(activateTerminalLink("mailto:hello@example.test", () => true, open)).toBe(true);
    expect(open).toHaveBeenCalledTimes(2);
    open.mockClear();
    let answer!: (accepted: boolean) => void;
    activateTerminalLink("mailto:hello@example.test", () => new Promise((resolve) => { answer = resolve; }), open);
    expect(open).not.toHaveBeenCalled();
    answer(true);
    await vi.waitFor(() => expect(open).toHaveBeenCalledTimes(1));
    open.mockClear();
    activateTerminalLink("mailto:hello@example.test", () => Promise.resolve(false), open);
    await Promise.resolve();
    expect(open).not.toHaveBeenCalled();
  });
});
