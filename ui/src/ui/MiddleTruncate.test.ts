import { describe, expect, it } from "vitest";
import { splitLabel } from "./MiddleTruncate";

describe("middle truncation", () => {
  it("keeps the part that tells similar names apart", () => {
    expect(splitLabel("prod-api-east")).toEqual(["prod-api", "-east"]);
    expect(splitLabel("prod-db-primary")).toEqual(["prod-db", "-primary"]);
    expect(splitLabel("build-01 · Pane 2")).toEqual(["build-01", " · Pane 2"]);
    expect(splitLabel("Terminal 7")).toEqual(["Terminal", " 7"]);
    expect(splitLabel("Raspberry Pi 家庭网关")).toEqual(["Raspberry Pi", " 家庭网关"]);
  });

  it("leaves names without a short last part to truncate at the end", () => {
    expect(splitLabel("staging")).toEqual(["staging", ""]);
    expect(splitLabel("api-internalservicename")).toEqual(["api-internalservicename", ""]);
    expect(splitLabel("-x")).toEqual(["-x", ""]);
  });
});
