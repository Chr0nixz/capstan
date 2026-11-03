import { describe, expect, it } from "vitest";
import { searchCommands } from "./registry";

describe("registered command search", () => {
  const actions = [
    { key: "host:1", label: "Connect to Build Server", keywords: ["host", "connect", "build.example.test"] },
    { key: "snippet:1", label: "Run List pods", keywords: ["snippet", "kubectl", "operations"] },
    { key: "theme:light", label: "Switch to Light theme", keywords: ["theme", "appearance"] },
  ];

  it("matches localized labels, entity names, and multiword queries", () => {
    expect(searchCommands(actions, "build server").map((action) => action.key)).toEqual(["host:1"]);
    expect(searchCommands(actions, "kubectl list").map((action) => action.key)).toEqual(["snippet:1"]);
    expect(searchCommands(actions, "light appearance").map((action) => action.key)).toEqual(["theme:light"]);
  });

  it("returns the explicit action set for an empty query and never interprets search as execution", () => {
    expect(searchCommands(actions, " ")).toEqual(actions);
    expect(searchCommands(actions, "whoami")).toEqual([]);
  });
});
