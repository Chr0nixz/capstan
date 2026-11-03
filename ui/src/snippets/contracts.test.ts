import { describe, expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M3-contract.json";
import { decodeSnippet, decodeSnippetGroup } from "./contracts";

describe("snippet contract", () => {
  it("keeps the encrypted record shape metadata-only and accepts template defaults", () => {
    const group = decodeSnippetGroup(fixture.snippetGroups[0]);
    const snippet = decodeSnippet(fixture.snippets[0], new Set([group.id]));
    expect(snippet.template).toBe("kubectl get pods -n {{namespace:default}}");
    expect(snippet.auto_enter).toBe(true);
  });

  it.each(fixture.snippetTextCases)("matches Rust UTF-8 text validation for $name", ({ field, text, repeat, suffix, accepted }) => {
    const content = text.repeat(repeat) + suffix;
    const groups = new Set(fixture.snippetGroups.map((group) => group.id));
    const decode = field === "group_name"
      ? () => decodeSnippetGroup({ ...fixture.snippetGroups[0], name: content })
      : () => decodeSnippet({ ...fixture.snippets[0], [field]: content }, groups);
    if (accepted) expect(decode()).toBeDefined();
    else expect(decode).toThrow("snippet_contract_invalid");
  });

  it("rejects unpaired surrogates and nil identities just as Rust JSON and model validation do", () => {
    const nil = "00000000-0000-0000-0000-000000000000";
    const groups = new Set([nil, fixture.snippetGroups[0].id]);
    for (const text of ["\ud800", "\udfff", "ok\ud800text"]) {
      expect(() => decodeSnippet({ ...fixture.snippets[0], template: text }, groups)).toThrow("snippet_contract_invalid");
      expect(() => decodeSnippetGroup({ ...fixture.snippetGroups[0], name: text })).toThrow("snippet_contract_invalid");
    }
    for (const key of ["id", "group_id"]) {
      expect(() => decodeSnippet({ ...fixture.snippets[0], [key]: nil }, groups)).toThrow("snippet_contract_invalid");
    }
    expect(() => decodeSnippetGroup({ ...fixture.snippetGroups[0], id: nil })).toThrow("snippet_contract_invalid");
  });

  it("rejects unknown groups, oversized content, control names and extra fields", () => {
    const groups = new Set(fixture.snippetGroups.map((group) => decodeSnippetGroup(group).id));
    for (const value of [
      { ...fixture.snippets[0], group_id: "77777777-7777-4777-8777-777777777777" },
      { ...fixture.snippets[0], template: "x".repeat(8193) },
      { ...fixture.snippets[0], name: "bad\nname" },
      { ...fixture.snippets[0], command: "must not cross the boundary" },
    ]) expect(() => decodeSnippet(value, groups)).toThrow("snippet_contract_invalid");
  });
});
