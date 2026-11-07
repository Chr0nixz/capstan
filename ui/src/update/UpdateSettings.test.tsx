import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M4-contract.json";
import { copy } from "../i18n";
import { UpdateCenter, UpdateSettings } from "./UpdateSettings";
import { useUpdates } from "./store";
import type { UpdateStatus } from "./contracts";

Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
let container: HTMLDivElement;
let root: Root;
let status: UpdateStatus;
const invoke = vi.fn(async (command: string, _args?: Record<string, unknown>): Promise<unknown> => {
  if (command === "update_activity") return false;
  return status;
});
beforeEach(() => {
  status = { ...fixture.updateStatus } as UpdateStatus;
  useUpdates.setState({ status: null, dialogOpen: false, saving: false, preferenceError: false, loadError: false });
  invoke.mockClear();
  invoke.mockImplementation(async (command) => command === "update_activity" ? false : status);
  window.__TAURI__ = { core: { invoke: <T,>(command: string, args?: Record<string, unknown>) => invoke(command, args) as Promise<T>, Channel: class { onmessage = null; } } };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); delete window.__TAURI__; });

async function render(): Promise<void> {
  await act(async () => root.render(<><UpdateCenter locale="en" /><UpdateSettings locale="en" /></>));
}
function button(label: string): HTMLButtonElement {
  const found = [...document.querySelectorAll("button")].find((item) => item.textContent === label);
  if (!found) throw new Error(`Missing button: ${label}`);
  return found;
}

describe("software updates", () => {
  it("reads the shared contract and disables checks without a trusted key", async () => {
    expect(fixture.updateTarget).toBe("windows-x86_64");
    expect(fixture.updateLimits.packageBytes).toBe(200 * 1024 * 1024);
    await render();
    expect(container.textContent).toContain(copy.en.update.unavailable);
    expect(button(copy.en.update.check).disabled).toBe(true);
    expect(document.querySelector<HTMLInputElement>('input[type="checkbox"]')?.checked).toBe(false);
    expect(invoke).not.toHaveBeenCalledWith("check_update", expect.anything());
  });

  it("renders release notes as text and waits for explicit installation", async () => {
    status = { ...status, available: true, phase: "available", release: { id: "release-id", version: "1.2.3", notes: '<img src="https://untrusted.test/a" onerror="alert(1)">' } };
    await render();
    await act(async () => button(copy.en.update.details).click());
    expect(document.querySelector(".update-notes")?.textContent).toContain("<img");
    expect(document.querySelector(".update-notes img")).toBeNull();
    expect(document.activeElement?.textContent).toBe(copy.en.cancelEdit);
    expect(invoke.mock.calls.some(([command]) => command === "install_update")).toBe(false);
    await act(async () => button(copy.en.update.install).click());
    expect(invoke).toHaveBeenCalledWith("install_update", { id: "release-id", confirmed: true, confirmActivity: false });
  });

  it("requires an extra confirmation for active sessions and Escape cancels it", async () => {
    status = { ...status, available: true, phase: "available", release: { id: "release-id", version: "1.2.3", notes: "Notes" } };
    invoke.mockImplementation(async (command) => command === "update_activity" ? true : status);
    await render();
    await act(async () => button(copy.en.update.details).click());
    await act(async () => button(copy.en.update.install).click());
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain(copy.en.update.confirmActivity);
    await act(async () => document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(invoke.mock.calls.some(([command]) => command === "install_update")).toBe(false);
  });

  it("keeps portable builds announcement-only", async () => {
    status = { ...status, available: true, portable: true, phase: "available", release: { id: "release-id", version: "1.2.3", notes: "Notes" } };
    await render();
    await act(async () => button(copy.en.update.details).click());
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain(copy.en.update.portable);
    expect([...document.querySelectorAll("button")].some((item) => item.textContent === copy.en.update.install)).toBe(false);
  });

  it("cancels downloads and exposes signature failures without losing the retry action", async () => {
    status = { ...status, available: true, phase: "downloading", downloaded: 50, total: 100, release: { id: "release-id", version: "1.2.3", notes: "Notes" } };
    await render();
    await act(async () => button(copy.en.update.details).click());
    expect(document.querySelector("progress")?.value).toBe(50);
    await act(async () => button(copy.en.update.cancelDownload).click());
    expect(invoke).toHaveBeenCalledWith("cancel_update", undefined);
    await act(async () => useUpdates.setState({ status: { ...status, phase: "error", errorCode: "update_signature" } }));
    expect(document.querySelector('.update-dialog [role="alert"]')?.textContent).toBe(copy.en.update.signature);
    expect(button(copy.en.update.install).disabled).toBe(false);
  });
});
