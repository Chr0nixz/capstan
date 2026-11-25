import { act } from "react";
import { createRoot } from "react-dom/client";
import { createStore } from "zustand/vanilla";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CWD_HINT_PATIENCE_MS, CwdControls } from "./CwdControls";
import type { createCwdController } from "./cwd";

type Controller = ReturnType<typeof createCwdController>;

function fakeController(): Controller {
  const store = createStore(() => ({ follow: true, snapshot: null, error: null, errorSource: null, saving: false }));
  return { store, aliases: vi.fn(async () => undefined), setFollow: vi.fn() } as unknown as Controller;
}

describe("follow waiting for directory hints", () => {
  let container: HTMLDivElement;
  beforeEach(() => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    vi.useFakeTimers();
    container = document.createElement("div");
    document.body.append(container);
  });
  afterEach(() => {
    vi.useRealTimers();
    container.remove();
  });

  it("offers shell integration in the startup files it found once the wait runs long", async () => {
    const root = createRoot(container);
    const setup = vi.fn();
    const files = [{ name: ".bashrc", shell: "bash" as const }, { name: ".zshrc", shell: "zsh" as const }];
    await act(async () => { root.render(<CwdControls controller={fakeController()} locale="en" aliasesOpen={false} onAliasesOpenChange={() => undefined}
      integrationFiles={files} onSetupIntegration={setup} />); });
    expect(container.querySelector('[role="status"]')?.textContent).toBe("Waiting for shell directory hints");
    expect(container.querySelector(".sftp-cwd-setup")).toBeNull();
    await act(async () => { vi.advanceTimersByTime(CWD_HINT_PATIENCE_MS); });
    expect(container.querySelector('[role="status"]')?.textContent).toBe("This shell isn't reporting its directory.");
    const buttons = [...container.querySelectorAll<HTMLButtonElement>(".sftp-cwd-setup")];
    expect(buttons.map((button) => button.textContent)).toEqual(["Set up in .bashrc…", "Set up in .zshrc…"]);
    await act(async () => buttons[1].click());
    expect(setup).toHaveBeenCalledWith(files[1]);
    // Server hostnames stays available as the secondary fix.
    expect(container.querySelector(".sftp-cwd-aliases summary")?.textContent).toBe("Server hostnames");
    await act(async () => { root.unmount(); });
  });

  it("explains the manual route when no startup file is listed", async () => {
    const root = createRoot(container);
    await act(async () => { root.render(<CwdControls controller={fakeController()} locale="en" aliasesOpen={false} onAliasesOpenChange={() => undefined} onSetupIntegration={() => undefined} />); });
    await act(async () => { vi.advanceTimersByTime(CWD_HINT_PATIENCE_MS); });
    expect(container.querySelector('[role="status"]')?.textContent).toContain("Select .bashrc or .zshrc in your home folder");
    expect(container.querySelector(".sftp-cwd-setup")).toBeNull();
    await act(async () => { root.unmount(); });
  });
});
