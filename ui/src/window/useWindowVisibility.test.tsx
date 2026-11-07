import { act, useCallback } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useWindowVisibility } from "./useWindowVisibility";

const events = vi.hoisted(() => new Map<string, (value: unknown) => void>());
vi.mock("../terminal/bridge", () => ({
  isTauriAvailable: () => true,
  listenTauriEvent: async (name: string, handler: (value: unknown) => void) => {
    events.set(name, handler); return () => events.delete(name);
  },
}));
Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });

it("propagates native hide/show to rendering and restores focus without replacing the workspace", async () => {
  const shown = vi.fn(), fallback = vi.fn();
  function Probe() {
    const visible = useWindowVisibility(useCallback(shown, []), useCallback(fallback, []));
    return <span data-visible={visible} />;
  }
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<Probe />));
  await act(async () => events.get("quake-visibility-changed")?.(false));
  expect(container.firstElementChild?.getAttribute("data-visible")).toBe("false");
  expect(shown).not.toHaveBeenCalled();
  await act(async () => events.get("quake-visibility-changed")?.(true));
  expect(container.firstElementChild?.getAttribute("data-visible")).toBe("true");
  expect(shown).toHaveBeenCalledTimes(1);
  await act(async () => events.get("launch-cwd-fallback")?.(null));
  expect(fallback).toHaveBeenCalledTimes(1);
  await act(async () => root.unmount());
  expect(events.size).toBe(0);
});
