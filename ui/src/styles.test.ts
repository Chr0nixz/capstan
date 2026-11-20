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
