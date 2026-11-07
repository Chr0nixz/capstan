import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M4-contract.json";
import { copy } from "../i18n";
import { decodeServerStatus } from "./contracts";
import { ServerStatusBar } from "./ServerStatusBar";

const { retry } = vi.hoisted(() => ({ retry: vi.fn(async () => undefined) }));
vi.mock("./useServerStatus", () => ({ retryServerStatus: retry }));
Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

it("exposes metrics and restores focus when Escape closes details", () => {
  act(() => root.render(<ServerStatusBar host="devbox" snapshot={decodeServerStatus(fixture.serverStatus)} labels={copy.en.serverStatus} />));
  const button = container.querySelector<HTMLButtonElement>(".server-status-summary")!;
  const details = container.querySelector<HTMLDivElement>(".server-status-details")!;
  expect(button.textContent).toContain("CPU 26%");
  expect(button.textContent).toContain("Disk 2.0 KiB / 8.0 KiB");
  expect(details.hidden).toBe(true);
  act(() => button.click());
  expect(button.getAttribute("aria-expanded")).toBe("true");
  expect(details.hidden).toBe(false);
  expect(details.textContent).toContain("eth0");
  details.focus();
  act(() => details.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
  expect(document.activeElement).toBe(button);
  expect(button.getAttribute("aria-expanded")).toBe("false");
});

it("offers retry for a failed collector with its current generation", () => {
  const snapshot = decodeServerStatus({ connectionId: fixture.serverStatus.connectionId, generation: fixture.serverStatus.generation,
    sampledAt: "0", state: "unavailable", errorCode: "status_rejected" });
  act(() => root.render(<ServerStatusBar host="devbox" snapshot={snapshot} labels={copy.en.serverStatus} />));
  act(() => container.querySelector<HTMLButtonElement>(".server-status-summary")!.click());
  const details = container.querySelector<HTMLDivElement>(".server-status-details")!;
  expect(details.textContent).toContain("The server refused the status channel.");
  act(() => details.querySelector("button")!.click());
  expect(retry).toHaveBeenCalledWith(snapshot);
});
