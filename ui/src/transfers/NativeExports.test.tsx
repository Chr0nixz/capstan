import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { NativeExports } from "./NativeExports";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../terminal/bridge", () => ({ isTauriAvailable: () => true, invokeTauri: (command: string, args: unknown) => mocks.invoke(command, args) }));
describe("retained Explorer copies", () => {
  it("requires explicit copy-complete consent and preserves a copy when native cleanup fails", async () => {
    Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
    const item = { token: "00000000-0000-4000-8000-000000000006", name: "folder", state: "retained" };
    mocks.invoke.mockImplementation(async (command: string) => { if (command === "native_export_snapshot") return [item]; if (command === "release_native_export") throw { code: "local_io" }; return undefined; });
    const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
    const button = (text: string): HTMLButtonElement => [...document.querySelectorAll("button")].find((button) => button.textContent === text)!;
    try {
      await act(async () => root.render(<NativeExports locale="en" theme="graphite" />));
      await act(async () => button("Explorer exports · 1").click());
      expect(mocks.invoke.mock.calls.some(([command]) => command === "release_native_export")).toBe(false);
      await act(async () => button("Remove export copy").click());
      expect(document.body.textContent).toContain("Wait for Explorer to finish copying");
      await act(async () => button("Keep open").click());
      expect(mocks.invoke.mock.calls.some(([command]) => command === "release_native_export")).toBe(false);
      await act(async () => button("Remove export copy").click());
      await act(async () => button("Copy finished — remove").click());
      expect(mocks.invoke).toHaveBeenCalledWith("release_native_export", { token: item.token });
      expect(document.body.textContent).toContain("folder");
      expect(document.querySelector('[role="alert"]')).not.toBeNull();
    } finally { await act(async () => root.unmount()); container.remove(); mocks.invoke.mockReset(); }
  });
});
