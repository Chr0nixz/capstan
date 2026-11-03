import { describe, expect, it, vi } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-editor.json";
import { decodeRemoteEditSnapshot } from "../sftp/contracts";
import { createEditorController } from "./controller";
import type { EditorTransport } from "./transport";

function setup() {
  let current = decodeRemoteEditSnapshot(fixture);
  const transport: EditorTransport = {
    snapshot: vi.fn(async () => structuredClone(current)), discover: vi.fn(async () => undefined), configure: vi.fn(async () => null),
    retry: vi.fn(async () => current.sessions[0]), cleanup: vi.fn(async () => current.sessions[0]), resolve: vi.fn(async () => current.sessions[0]), close: vi.fn(async () => false),
  };
  return { transport, controller: createEditorController(transport), setSnapshot: (snapshot: typeof current) => { current = snapshot; } };
}
describe("editor controller", () => {
  it("keeps a session when the export picker is canceled or cleanup fails", async () => {
    const { controller, transport } = setup();
    await controller.refresh();
    const id = fixture.sessions[0].id;
    expect(await controller.close(id, "save_local_copy", "en")).toBe(false);
    expect(controller.store.getState().snapshot?.sessions).toHaveLength(2);
    vi.mocked(transport.close).mockRejectedValueOnce({ code: "editor_residual" });
    await expect(controller.close(id, "discard", "en")).rejects.toEqual({ code: "editor_residual" });
    expect(controller.store.getState().error).toBe("editor_residual");
    expect(controller.store.getState().snapshot?.sessions).toHaveLength(2);
  });
  it("deduplicates polling and rejects older revision snapshots", async () => {
    const { controller, transport } = setup();
    let resolve: ((value: ReturnType<typeof decodeRemoteEditSnapshot>) => void) | undefined;
    vi.mocked(transport.snapshot).mockReturnValueOnce(new Promise((done) => { resolve = done; }));
    const first = controller.refresh(), second = controller.refresh();
    expect(first).toBe(second);
    expect(transport.snapshot).toHaveBeenCalledTimes(1);
    resolve?.(decodeRemoteEditSnapshot(fixture));
    await first;
    vi.mocked(transport.snapshot).mockResolvedValueOnce(decodeRemoteEditSnapshot({ ...fixture, revision: "1", sessions: [] }));
    await controller.refresh();
    expect(controller.store.getState().snapshot?.sessions).toHaveLength(2);
  });
  it("does not resurrect a closed edit from an earlier in-flight snapshot", async () => {
    const { controller, transport, setSnapshot } = setup();
    await controller.refresh();
    let release: ((value: ReturnType<typeof decodeRemoteEditSnapshot>) => void) | undefined;
    vi.mocked(transport.snapshot).mockReturnValueOnce(new Promise((done) => { release = done; }));
    const poll = controller.refresh();
    vi.mocked(transport.close).mockImplementationOnce(async () => {
      setSnapshot(decodeRemoteEditSnapshot({ ...fixture, revision: "9007199254740994", sessions: [fixture.sessions[1]] }));
      return true;
    });
    const closed = controller.close(fixture.sessions[0].id, "discard", "en");
    await Promise.resolve();
    release?.(decodeRemoteEditSnapshot(fixture));
    await Promise.all([poll, closed]);
    expect(controller.store.getState().snapshot?.sessions.map((entry) => entry.id)).toEqual([fixture.sessions[1].id]);
  });
});
