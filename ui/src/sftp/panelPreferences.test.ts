import { describe, expect, it, vi } from "vitest";
import { createPanelPreferences, decodePanelLayout, type PanelLayout } from "./panelPreferences";
import type { ExactU64 } from "./contracts";

const session = "00000000-0000-4000-8000-000000000001";
const original: PanelLayout = { dock: "right", size: 36, collapsed: false };
const changed: PanelLayout = { dock: "bottom", size: 55, collapsed: true };
const snapshot = (layout = original, sequence = "0") => ({ session, layout, sequence: sequence as ExactU64 });
const settle = async (): Promise<void> => { await Promise.resolve(); await Promise.resolve(); };

describe("nonsecret panel preferences", () => {
  it("loads only once under StrictMode, initializes current tabs and preserves their independent layouts", async () => {
    const transport = { load: vi.fn(async () => snapshot(changed)), save: vi.fn(async () => undefined) };
    const preferences = createPanelPreferences(transport);
    preferences.prune(["one", "two"]);
    await Promise.all([preferences.initialize(), preferences.initialize()]);
    expect(transport.load).toHaveBeenCalledTimes(1);
    expect(preferences.store.getState().tabs.two).toEqual(changed);
    preferences.set("one", original);
    expect(preferences.store.getState().tabs.two).toEqual(changed);
    preferences.prune(["one", "two", "new"]);
    expect(preferences.store.getState().tabs.new).toEqual(original);
    await settle();
    expect(transport.save).toHaveBeenCalledExactlyOnceWith({ session, sequence: "1", layout: original });
    preferences.prune(["new"]);
    expect(Object.keys(preferences.store.getState().tabs)).toEqual(["new"]);
  });

  it("does not overwrite a user change with late hydration and sends only the fixed preference DTO", async () => {
    let finish!: (value: ReturnType<typeof snapshot>) => void;
    const transport = { load: vi.fn(() => new Promise<ReturnType<typeof snapshot>>((resolve) => { finish = resolve; })), save: vi.fn(async (_value: ReturnType<typeof snapshot>) => undefined) };
    const preferences = createPanelPreferences(transport);
    const loading = preferences.initialize();
    preferences.prune(["one"]); preferences.set("one", changed);
    finish(snapshot(original, "9007199254740993")); await loading; await settle();
    expect(preferences.store.getState().layout).toEqual(changed);
    expect(transport.save).toHaveBeenCalledExactlyOnceWith({ session, sequence: "9007199254740994", layout: changed });
    expect(Object.keys(transport.save.mock.calls[0][0]).sort()).toEqual(["layout", "sequence", "session"]);
  });

  it("ignores old save failures after newer writes succeed", async () => {
    let fail!: (error: Error) => void;
    const transport = { load: vi.fn(async () => snapshot()), save: vi.fn(async (_value: ReturnType<typeof snapshot>) => undefined) };
    const preferences = createPanelPreferences(transport); await preferences.initialize();
    transport.save.mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
    preferences.set("one", changed); preferences.set("one", original); await settle();
    fail(new Error("disk full")); await settle();
    expect(preferences.store.getState().error).toBeNull();
    expect(transport.save.mock.calls.map((call) => call[0].sequence)).toEqual(["1", "2"]);
  });

  it("keeps session layout on load/save failures and retries without replacing it", async () => {
    const transport = { load: vi.fn(async () => snapshot()), save: vi.fn(async () => undefined) };
    transport.load.mockRejectedValueOnce(new Error("invalid settings"));
    const preferences = createPanelPreferences(transport); await preferences.initialize();
    expect(preferences.store.getState().error).toBe("load");
    preferences.set("one", changed);
    transport.save.mockRejectedValueOnce(new Error("permission denied"));
    await preferences.retry(); await settle();
    expect(preferences.store.getState().layout).toEqual(changed);
    expect(preferences.store.getState().error).toBe("save");
    await preferences.retry(); await settle();
    expect(preferences.store.getState().error).toBeNull();
  });

  it("fails instead of wrapping an exhausted exact sequence", async () => {
    const transport = { load: vi.fn(async () => snapshot(original, "18446744073709551615")), save: vi.fn(async () => undefined) };
    const preferences = createPanelPreferences(transport); await preferences.initialize(); preferences.set("one", changed);
    expect(transport.save).not.toHaveBeenCalled();
    expect(preferences.store.getState().error).toBe("save");
  });

  it("rejects unknown fields, unsupported docks and fractional/out-of-range sizes", () => {
    for (const value of [null, { ...original, dock: "left" }, { ...original, size: 61 }, { ...original, size: 20.5 }, { ...original, path: "/private" }]) {
      expect(() => decodePanelLayout(value)).toThrow("settings_invalid");
    }
  });
});
