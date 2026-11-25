/// <reference types="vite/client" />
import { describe, expect, it } from "vitest";
import css from "./styles.css?raw";

const defined = new Set([...css.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((match) => match[1]));
const rootBlock = css.slice(css.indexOf(":root {"), css.indexOf("}", css.indexOf(":root {")));
const rootTokens = new Set([...rootBlock.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((match) => match[1]));
const forcedStart = css.indexOf("@media (forced-colors: active) {\n  :root, [data-theme] {");
const forcedTokens = css.slice(forcedStart, css.indexOf("}", forcedStart + 40)).match(/--[a-z0-9-]+(?=\s*:)/g) ?? [];

function rule(selector: string): string {
  const start = css.indexOf(selector);
  expect(start, selector).toBeGreaterThanOrEqual(0);
  return css.slice(start, css.indexOf("}", start));
}

describe("design tokens", () => {
  it("defines every custom property the stylesheet reads without a fallback", () => {
    // Properties read with a fallback (such as --sftp-size) are set inline by components.
    const used = new Set([...css.matchAll(/var\((--[a-z0-9-]+)\s*\)/g)].map((match) => match[1]));
    expect([...used].filter((token) => !defined.has(token))).toEqual([]);
  });

  it("gives every forced-colors override a normal value, so state rules can use the token", () => {
    expect(forcedTokens.length).toBeGreaterThan(20);
    expect(forcedTokens.filter((token) => !rootTokens.has(token))).toEqual([]);
  });

  it("keeps the primary key's hover and press on tokens that forced colors can replace", () => {
    expect(rule('.form-actions button[type="submit"]:hover:not(:disabled) {')).toContain("var(--amber-hover)");
    expect(rule('.form-actions button[type="submit"]:active:not(:disabled) {')).toContain("var(--amber-pressed)");
    expect(rule(".danger-action:hover:not(:disabled) {")).not.toMatch(/rgb\(|#[0-9a-f]{3,6}\b/i);
  });
});

function block(selector: string): Record<string, string> {
  const start = css.indexOf(selector);
  const body = css.slice(start, css.indexOf("}", start));
  return Object.fromEntries([...body.matchAll(/(--[a-z0-9-]+)\s*:\s*(#[0-9a-f]{6})\s*;/gi)].map((match) => [match[1], match[2]]));
}

function contrast(foreground: string, background: string): number {
  const luminance = (hex: string): number => {
    const [r, g, b] = [1, 3, 5].map((index) => parseInt(hex.slice(index, index + 2), 16) / 255)
      .map((value) => value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [light, dark] = [luminance(foreground), luminance(background)].sort((a, b) => b - a);
  return (light + 0.05) / (dark + 0.05);
}

/** Innermost rules as [selector, declarations]; @media wrappers are skipped, their rules kept. */
const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((match) => [match[1].trim(), match[2]] as const);

describe("focus and highlighted rows", () => {
  it("keeps the amber focus key on keyboard focus inside menus", () => {
    // Hover may share the face, but focus-visible must never drop its outline.
    const menuFocus = rules.filter(([selector]) => /menu/.test(selector) && selector.includes(":focus-visible"));
    expect(menuFocus.length).toBeGreaterThan(0);
    expect(menuFocus.filter(([, body]) => /outline:\s*none/.test(body)).map(([selector]) => selector)).toEqual([]);
  });

  it("marks the palette's active row with more than the hover face", () => {
    expect(rule('.command-result[aria-selected="true"] {')).toContain("inset 0 0 0 1px var(--amber)");
  });

  it("steps faint text up to Muted wherever a row takes the hover or selected face", () => {
    expect(rule('.command-result[aria-selected="true"] .command-icon, .command-result[aria-selected="true"] .command-detail {')).toContain("var(--legend-muted)");
    expect(rule(".sftp-table tbody tr:hover td:nth-child(3) .mono, .sftp-table tr.selected td:nth-child(3) .mono {")).toContain("var(--legend-muted)");
    expect(rule(".sftp-menu button:hover:not(:disabled) .sftp-menu-label[data-keys]::after,")).toContain("var(--legend-muted)");
  });

  it("keeps Muted text at 4.5:1 on the hover and selected faces in both colorways", () => {
    for (const tokens of [block(":root {"), block('[data-theme="light"] {')]) {
      for (const face of ["--face-hover", "--selected"]) {
        expect(contrast(tokens["--legend-muted"], tokens[face]), face).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
});

describe("stacked remote files", () => {
  it("never lets the stacked panel's minimum take more than half the height", () => {
    expect(rule(".terminal-pane-sftp-bottom { display: grid;")).toContain("min(300px, 50%)");
    expect(rule("  .terminal-pane-sftp-right { grid-template-columns: minmax(0, 1fr);")).toContain("min(300px, 50%)");
  });
});
