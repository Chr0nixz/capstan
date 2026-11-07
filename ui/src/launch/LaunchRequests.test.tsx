import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { LaunchRequests } from "./LaunchRequests";
import type { LaunchRequestView } from "../terminal/contracts";

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
vi.mock("../terminal/bridge", () => ({
  isTauriAvailable: () => true, invokeTauri: bridge.invoke, listenTauriEvent: bridge.listen,
}));

let root: Root;
let container: HTMLDivElement;
let receive: (value: unknown) => void;
const first: LaunchRequestView = { requestId: "first", kind: "ssh", display: "user@host:22" };
const second: LaunchRequestView = { requestId: "second", kind: "local", display: "" };
beforeEach(() => {
  bridge.invoke.mockReset();
  bridge.listen.mockReset();
  bridge.invoke.mockResolvedValue([]);
  bridge.listen.mockImplementation(async (_name: string, handler: (value: unknown) => void) => {
    receive = handler;
    return () => undefined;
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

it("subscribes before listing requests and handles a raced event exactly once", async () => {
  const order: string[] = [];
  bridge.listen.mockImplementation(async (_name: string, handler: (value: unknown) => void) => {
    order.push("listen"); receive = handler; handler(first); return () => undefined;
  });
  bridge.invoke.mockImplementation(async () => { order.push("list"); return [first]; });
  const handle = vi.fn(async () => undefined);
  await act(async () => root.render(<LaunchRequests onRequest={handle} />));
  expect(order.slice(0, 2)).toEqual(["listen", "list"]);
  expect(handle).toHaveBeenCalledExactlyOnceWith(first);
  await act(async () => { receive(first); receive({ kind: "ssh", requestId: 4 }); });
  expect(handle).toHaveBeenCalledTimes(1);
});

it("keeps confirmations serialized and advances when a canceled request completes", async () => {
  let finish: (() => void) | undefined;
  const handle = vi.fn(async (request: LaunchRequestView) => {
    if (request.requestId === first.requestId) await new Promise<void>((resolve) => { finish = resolve; });
  });
  await act(async () => root.render(<LaunchRequests onRequest={handle} />));
  await act(async () => { receive(first); receive(second); receive(first); });
  expect(handle).toHaveBeenCalledExactlyOnceWith(first);
  await act(async () => finish?.());
  expect(handle.mock.calls.map(([request]) => request.requestId)).toEqual(["first", "second"]);
});

it("continues after a rejected request without replaying it from the snapshot", async () => {
  bridge.invoke.mockResolvedValue([first, second]);
  const handle = vi.fn(async (request: LaunchRequestView) => {
    if (request.requestId === first.requestId) throw new Error("canceled");
  });
  await act(async () => root.render(<LaunchRequests onRequest={handle} />));
  expect(handle.mock.calls.map(([request]) => request.requestId)).toEqual(["first", "second"]);
});
