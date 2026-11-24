import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { DiagnosticsSettings } from "./DiagnosticsSettings";
import { copy } from "../i18n";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../terminal/bridge", () => ({ invokeTauri: invoke }));
Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
let root: Root;
let container: HTMLDivElement;
beforeEach(() => {
  invoke.mockReset();
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

it("asks Rust to show the log folder without sending a path, and reports a failure", async () => {
  await act(async () => root.render(<DiagnosticsSettings locale="en" />));
  expect(container.textContent).toContain(copy.en.diagnosticsHint);
  invoke.mockResolvedValueOnce(undefined);
  await act(async () => container.querySelector("button")!.click());
  expect(invoke).toHaveBeenCalledExactlyOnceWith("open_diagnostics_folder");
  expect(container.querySelector("[role='alert']")).toBeNull();
  invoke.mockRejectedValueOnce({ code: "diagnostics_unavailable" });
  await act(async () => container.querySelector("button")!.click());
  expect(container.querySelector("[role='alert']")?.textContent).toBe(copy.en.diagnosticsError);
  expect(container.querySelector("button")!.disabled).toBe(false);
});
