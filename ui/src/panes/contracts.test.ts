import { describe, expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M3-contract.json";
import { confirmInputRoute, decodePaneLayout, routeInput } from "./contracts";

describe("pane layout contract", () => {
  it("decodes nested serializable panes without turning a pane into a terminal session", () => {
    const layout = decodePaneLayout(fixture.paneLayout);
    expect(layout.tabId).toBe("tab-7");
    expect(layout.root.kind).toBe("split");
    expect(layout.root.kind === "split" && layout.root.second.kind).toBe("split");
  });

  it("rejects duplicate pane IDs, malformed launches, ratios and unknown fields", () => {
    const root = fixture.paneLayout.root;
    const split = root.second;
    for (const value of [
      { ...fixture.paneLayout, root: { ...root, second: { ...split, second: { ...split.second, paneId: "pane-a" } } } },
      { ...fixture.paneLayout, root: { ...root, ratio: 1.1 } },
      { ...fixture.paneLayout, root: { ...root, direction: "diagonal" } },
      { ...fixture.paneLayout, root: { ...root, extra: true } },
      { ...fixture.paneLayout, root: { kind: "pane", paneId: "pane-x", launch: { kind: "shell", command: "whoami" } } },
    ]) expect(() => decodePaneLayout(value)).toThrow("pane_contract_invalid");
  });

  it("routes ordinary input to focus, broadcasts only to explicit targets and gates multiline input", () => {
    const focused = "pane-a" as import("./contracts").PaneId;
    const second = "pane-b" as import("./contracts").PaneId;
    expect(routeInput(focused, [], false, "date")).toEqual({
      targetPaneIds: [focused], requiresConfirmation: false, confirmation: null, blockedReason: null,
    });
    const multiline = routeInput(focused, [focused, second, focused], true, "one\ntwo");
    expect(multiline).toEqual({
      targetPaneIds: [focused, second], requiresConfirmation: true, confirmation: "multiline", blockedReason: null,
    });
    expect(confirmInputRoute(multiline, false)).toEqual([]);
    expect(confirmInputRoute(multiline, true)).toEqual([focused, second]);
    const empty = routeInput(focused, [], true, "date");
    expect(empty.blockedReason).toBe("no_targets");
    expect(confirmInputRoute(empty, true)).toEqual([]);
  });

  it.each(fixture.broadcastInputs)("gates $name input before delivery and cancels every target", ({ input, confirmation, armedConfirmation }) => {
    const focused = "pane-a" as import("./contracts").PaneId;
    const second = "pane-b" as import("./contracts").PaneId;
    const route = routeInput(focused, [focused, second, focused], true, input);
    expect(route.requiresConfirmation).toBe(confirmation);
    expect(confirmInputRoute(route, false)).toEqual([]);
    expect(confirmInputRoute(route, true)).toEqual([focused, second]);
    // Once this exact set is armed, only input spanning several lines asks again.
    expect(routeInput(focused, [focused, second], true, input, [second, focused]).requiresConfirmation).toBe(armedConfirmation);
    expect(routeInput(focused, [second], false, input)).toEqual({
      targetPaneIds: [focused], requiresConfirmation: false, confirmation: null, blockedReason: null,
    });
    expect(confirmInputRoute(routeInput(focused, [], true, input), true)).toEqual([]);
  });

  it("asks again when the selection differs from the armed set", () => {
    const [a, b, c] = ["pane-a", "pane-b", "pane-c"] as import("./contracts").PaneId[];
    expect(routeInput(a, [a, b], true, "\r", [a, b]).confirmation).toBeNull();
    expect(routeInput(a, [a, b, c], true, "\r", [a, b]).confirmation).toBe("arm");
    expect(routeInput(a, [a], true, "\r", [a, b]).confirmation).toBe("arm");
    expect(routeInput(a, [a, b], true, "ls\rpwd\r", [a, b]).confirmation).toBe("multiline");
    expect(routeInput(a, [a, b], true, "ls", []).confirmation).toBeNull();
  });
});
