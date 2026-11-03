import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-tree.json";
import { decodeTreePlan } from "./tree";
import { TreeTransferDialog } from "./TreeTransferDialog";
describe("folder decision dialog", () => {
  it("reviews only displayed directories and keeps type conflicts out of merge consent", async () => {
    Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container), onDecision = vi.fn(), onClose = vi.fn();
    const plan = decodeTreePlan({ ...fixture, conflicts: Array.from({ length: 60 }, (_, entry) => ({ entry, source: `folder/${entry}`, relative: `folder/${entry}`, directory: true, kind: "directory_exists" })) })!;
    try {
      await act(async () => root.render(<TreeTransferDialog plan={plan} hostLabel="Build server" locale="en" theme="graphite" pending={false} error={null} onDecision={onDecision} onClose={onClose} />));
      expect(document.body.textContent).toContain("Build server");
      expect(document.querySelectorAll(".transfer-directory-review li")).toHaveLength(50);
      const merge = [...document.querySelectorAll("button")].find((button) => button.textContent?.includes("Merge listed directories"))!;
      await act(async () => merge.click());
      expect(onDecision.mock.calls[0][0]).toEqual(plan.conflicts.slice(0, 50).map((item) => ({ entry: item.entry, decision: { kind: "merge" } })));
      await act(async () => root.render(<TreeTransferDialog plan={{ ...plan, conflicts: [{ ...plan.conflicts[0], kind: "type_conflict" }] }} hostLabel="Build server" locale="zh-CN" theme="light" pending={false} error={null} onDecision={onDecision} onClose={onClose} />));
      expect(document.body.textContent).toContain("目标类型不同");
      expect([...document.querySelectorAll("button")].some((button) => button.textContent?.includes("合并"))).toBe(false);
    } finally { await act(async () => root.unmount()); container.remove(); }
  });
});
