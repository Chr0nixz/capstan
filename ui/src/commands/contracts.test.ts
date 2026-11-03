import { describe, expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M3-contract.json";
import { decodeCommandAction } from "./contracts";

describe("command action contract", () => {
  it("accepts only the fixed typed local action set", () => {
    expect(fixture.commandActions.map(decodeCommandAction).map((action) => action.id)).toEqual([
      "connect_host", "run_snippet", "set_theme", "toggle_panel", "open_settings", "workspace",
    ]);
  });

  it.each([["run"], "whoami", "split", null, 1])("rejects workspace commands outside the fixed set: %j", (command) => {
    expect(() => decodeCommandAction({ ...fixture.commandActions[5], command })).toThrow("command_action_invalid");
  });

  it("does not accept search text as a command or arbitrary process input", () => {
    for (const value of [
      { id: "execute", command: "whoami" },
      { ...fixture.commandActions[0], command: "whoami" },
      { ...fixture.commandActions[0], path: "C:\\private" },
      { ...fixture.commandActions[0], hostId: "not-an-id" },
    ]) expect(() => decodeCommandAction(value)).toThrow("command_action_invalid");
  });

  it.each(fixture.commandInvalidPanels.map((panel) => ({ panel })))("rejects non-string panel discriminants: $panel", ({ panel }) => {
    expect(() => decodeCommandAction({ ...fixture.commandActions[3], panel })).toThrow("command_action_invalid");
  });
});
