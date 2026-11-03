import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SftpPanel, splitName } from "./SftpPanel";
import { cwdHints } from "./cwdHints";
import { sftpCopy } from "./copy";
import type { TerminalSessionId } from "../terminal/contracts";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../terminal/bridge", () => ({ invokeTauri: (command: string, args?: unknown) => mocks.invoke(command, args), listenTauriEvent: async () => () => undefined }));

const terminal = "00000000-0000-4000-8000-000000000010" as TerminalSessionId;
const attached = {
  sessionId: "00000000-0000-4000-8000-000000000001",
  hostId: "00000000-0000-4000-8000-000000000002",
  connectionGeneration: "7", path: "/home/test", atomicReplace: true, fsync: true, requestBytes: 32768,
};
const page = (path = "/home/test") => ({
  sessionId: attached.sessionId, requestRevision: "1", pageSequence: 0, path, hasMore: false, omittedEntries: 0,
  entries: [
    { id: "00000000-0000-4000-8000-000000000011", name: "folder", metadata: { kind: "directory", size: null, modifiedSeconds: null, permissions: 0o40700 } },
    { id: "00000000-0000-4000-8000-000000000012", name: ".env", metadata: { kind: "file", size: "8", modifiedSeconds: "1700000000", permissions: 0o100600 } },
    { id: "00000000-0000-4000-8000-000000000013", name: "notes.txt", metadata: { kind: "file", size: "12", modifiedSeconds: "1700000000", permissions: 0o100600 } },
  ],
});
const layout = { dock: "right" as const, size: 36, collapsed: false };

let container: HTMLDivElement;
let root: Root;
let onLayoutChange: ReturnType<typeof vi.fn>;

beforeEach(async () => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation(async (command: string, _args?: unknown): Promise<unknown> => {
    if (command === "attach_sftp") return attached;
    if (command === "list_remote_directory") return page();
    if (command === "prepare_remote_mutation") return {
      token: "confirmation-token", hostId: attached.hostId, path: "/home/test/notes.txt",
      metadata: page().entries[2].metadata, mutation: { kind: "delete" },
    };
    return undefined;
  });
  onLayoutChange = vi.fn();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  await act(async () => {
    root.render(<StrictMode><SftpPanel terminal={terminal} locale="en" hostLabel="Build server" theme="graphite" layout={layout} onLayoutChange={onLayoutChange} /></StrictMode>);
    await new Promise((resolve) => window.setTimeout(resolve, 0));
  });
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("SFTP panel", () => {
  const button = (text: string): HTMLButtonElement | undefined => [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((entry) => entry.textContent === text);
  it("refreshes a ready export through expiry and disables the stale drag control", async () => {
    const previous = mocks.invoke.getMockImplementation();
    const exportToken = "00000000-0000-4000-8000-000000000080", planToken = "00000000-0000-4000-8000-000000000081";
    let exportState = "ready";
    mocks.invoke.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "prepare_native_export") return {
        export: { token: exportToken, name: "notes.txt", state: "planning" },
        plan: { token: planToken, revision: "0", name: "notes.txt", files: 1, conflicts: [] },
      };
      if (command === "resolve_transfer_tree_plan") return { plan: null, queued: 1 };
      if (command === "native_export_snapshot") return [{ token: exportToken, name: "notes.txt", state: exportState }];
      return previous?.(command, args);
    });
    await act(async () => button("notes.txt")?.click());
    await act(async () => button("Drag out to Explorer…")?.click());
    await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 550)); });
    const ready = button("Drag to Explorer · notes.txt");
    expect(ready?.disabled).toBe(false);
    expect(ready?.draggable).toBe(true);
    exportState = "retained";
    await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 550)); });
    const retained = button("Retained copy · notes.txt");
    expect(retained?.disabled).toBe(true);
    expect(retained?.draggable).toBe(false);
    const calls = mocks.invoke.mock.calls.filter(([command]) => command === "native_export_snapshot").length;
    await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 550)); });
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "native_export_snapshot").length).toBe(calls);
    expect(mocks.invoke.mock.calls.some(([command]) => command === "start_native_export")).toBe(false);
  });
  it("routes directory removal through a bounded recursive scope confirmation", async () => {
    const previous = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation(async (command: string, args?: unknown) => command === "prepare_remote_tree_delete" ? {
      token: "00000000-0000-4000-8000-000000000090", hostId: attached.hostId, path: "/home/test/folder", files: 2, directories: 1, symlinks: 0,
    } : previous?.(command, args));
    await act(async () => button("folder")?.click());
    await act(async () => button("Delete")?.click());
    expect(document.querySelector('[role="alertdialog"]')?.textContent).toContain("2 files · 1 directories");
    expect(mocks.invoke).toHaveBeenCalledWith("prepare_remote_tree_delete", { terminal, session: attached.sessionId, entry: page().entries[0].id });
    expect(mocks.invoke.mock.calls.some(([command]) => command === "apply_remote_tree_delete")).toBe(false);
  });
  it("keeps file controls usable on preference failures and offers retry", async () => {
    const retry = vi.fn();
    await act(async () => root.render(<SftpPanel terminal={terminal} locale="en" hostLabel="Build server" theme="graphite" layout={layout} onLayoutChange={onLayoutChange} preferencesError="save" onRetryPreferences={retry} />));
    expect(container.textContent).toContain("Panel preferences could not be saved");
    await act(async () => button("Retry preferences")?.click());
    expect(retry).toHaveBeenCalledTimes(1);
    expect(button("Upload file")?.disabled).toBe(false);
    await act(async () => root.render(<SftpPanel terminal={terminal} locale="zh-CN" hostLabel="Build server" theme="light" layout={layout} onLayoutChange={onLayoutChange} preferencesError="load" onRetryPreferences={retry} />));
    expect(container.querySelector("[role='alert']")?.textContent).toContain("无法读取已保存的面板偏好");
    expect(button("重试")).toBeDefined();
    await act(async () => button("重试")?.click());
    expect(retry).toHaveBeenCalledTimes(2);
    await act(async () => root.render(<SftpPanel terminal={terminal} locale="en" hostLabel="Build server" theme="graphite" layout={{ ...layout, collapsed: true }} onLayoutChange={onLayoutChange} preferencesError="load" />));
    expect(container.querySelector(".sftp-panel-collapsed")?.getAttribute("aria-label")).toContain("Saved panel preferences could not be read");
  });

  it("opens the double-clicked text entry with Rust's configured editor ID", async () => {
    const previous = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
      if (command === "remote_editor_configuration") return "00000000-0000-4000-8000-000000000030";
      if (command === "begin_remote_edit") return { id: "00000000-0000-4000-8000-000000000031", name: "notes.txt", remotePath: "/home/test/notes.txt", hostId: attached.hostId,
        state: "waiting_for_stable_save", dirty: false, recovered: false, cleanupRequired: false, replaceAllowed: false, error: null };
      return previous?.(command, args);
    });
    await act(async () => {
      button("notes.txt")?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
      await Promise.resolve();
    });
    expect(mocks.invoke).toHaveBeenCalledWith("begin_remote_edit", { terminal, session: attached.sessionId, entry: page().entries[2].id, editorId: "00000000-0000-4000-8000-000000000030" });
    expect(mocks.invoke.mock.calls.some(([command]) => command === "select_remote_editor")).toBe(false);
  });

  it("follows parsed hints, stops on manual navigation, and resumes from the latest valid hint", async () => {
    const previous = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
      if (command === "update_remote_cwd") return { sequence: args?.sequence, path: String(args?.uri).replace("file://host", ""), aliases: [] };
      if (command === "list_remote_directory") return page(String(args?.path));
      return previous?.(command, args);
    });
    const publisher = cwdHints.open(terminal);
    try {
      await act(async () => { publisher.publish("file://host/first"); await new Promise((resolve) => setTimeout(resolve, 300)); });
      expect(container.querySelector<HTMLInputElement>(".sftp-path-form input")?.value).toBe("/first");
      await act(async () => button("Parent")?.click());
      expect(button(sftpCopy.en.followShort)?.getAttribute("aria-pressed")).toBe("false");
      await act(async () => { publisher.publish("file://host/latest"); await new Promise((resolve) => setTimeout(resolve, 300)); });
      expect(container.querySelector<HTMLInputElement>(".sftp-path-form input")?.value).toBe("/");
      await act(async () => button(sftpCopy.en.followShort)?.click());
      expect(container.querySelector<HTMLInputElement>(".sftp-path-form input")?.value).toBe("/latest");
    } finally { await act(async () => publisher.close()); }
  });

  it("holds its browse scope throughout a native picker and applies the newest hint afterward", async () => {
    const previous = mocks.invoke.getMockImplementation();
    let finish!: (value: null) => void;
    mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
      if (command === "select_transfer_source") return new Promise((resolve) => { finish = resolve; });
      if (command === "update_remote_cwd") return { sequence: args?.sequence, path: "/after-picker", aliases: [] };
      if (command === "list_remote_directory") return page(String(args?.path));
      return previous?.(command, args);
    });
    const publisher = cwdHints.open(terminal);
    try {
      await act(async () => button("Upload file")?.click());
      await act(async () => { publisher.publish("file://host/after-picker"); await new Promise((resolve) => setTimeout(resolve, 300)); });
      expect(container.querySelector<HTMLInputElement>(".sftp-path-form input")?.value).toBe("/home/test");
      await act(async () => finish(null));
      expect(container.querySelector<HTMLInputElement>(".sftp-path-form input")?.value).toBe("/after-picker");
    } finally { await act(async () => publisher.close()); }
  });

  it("opens only native picker commands and restricts download to regular files", async () => {
    expect(button("Download file")?.disabled).toBe(true);
    await act(async () => button("folder")?.click());
    expect(button("Download file")?.disabled).toBe(true);
    await act(async () => button("notes.txt")?.click());
    expect(button("Download file")?.disabled).toBe(false);
    mocks.invoke.mockResolvedValueOnce(null);
    await act(async () => button("Upload file")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("select_transfer_source", { terminal, session: attached.sessionId, parent: "/home/test", locale: "en" });
  });

  it("previews and applies shell integration without exposing configuration bytes", async () => {
    const previous = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>): Promise<unknown> => {
      if (command === "preview_shell_integration") return {
        token: "00000000-0000-4000-8000-000000000020", shell: args?.shell, action: args?.action, host: "example.com",
        sourceHash: "A".repeat(64), replacementHash: "B".repeat(64), addedLines: 20, removedLines: 0,
      };
      if (command === "apply_shell_integration") return { backupCreated: true, replacementHash: "B".repeat(64) };
      return previous?.(command, args);
    });
    await act(async () => { button("notes.txt")?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    expect(button("Shell integration")?.disabled).toBe(false);
    await act(async () => { button("Shell integration")?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    expect(document.body.textContent).toContain("Adds 20 lines and removes 0 lines");
    expect(mocks.invoke).toHaveBeenCalledWith("preview_shell_integration", {
      terminal, session: attached.sessionId, entry: page().entries[2].id, shell: "bash", action: "install",
    });
    await act(async () => button("Apply integration")?.click());
    expect(mocks.invoke).toHaveBeenCalledWith("apply_shell_integration", {
      terminal, session: attached.sessionId, entry: page().entries[2].id, token: "00000000-0000-4000-8000-000000000020",
    });
  });

  it("enters directory links with the keyboard and blocks link permission changes", async () => {
    const linked = page();
    linked.entries[0].metadata.kind = "symlink";
    mocks.invoke.mockResolvedValueOnce(linked);
    await act(async () => button("Refresh")?.click());
    const link = button("folder");
    await act(async () => link?.click());
    expect(button("Permissions")?.disabled).toBe(true);
    await act(async () => link?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(mocks.invoke).toHaveBeenCalledWith("list_remote_directory", { terminal, session: attached.sessionId, path: "/home/test/folder" });
  });

  it("marks omitted names without declaring an empty directory", async () => {
    mocks.invoke.mockResolvedValueOnce({ ...page(), entries: [], omittedEntries: 2 });
    await act(async () => button("Refresh")?.click());
    expect(container.textContent).toContain("2 unsupported entries omitted");
    expect(container.textContent).not.toContain("This directory is empty");
    expect(button("Next page")).toBeUndefined();
  });

  it("closes a failed confirmation so its consumed token cannot be submitted again", async () => {
    await act(async () => button("notes.txt")?.click());
    await act(async () => button("Delete")?.click());
    mocks.invoke.mockRejectedValueOnce({ code: "target_conflict" });
    expect(button("Delete file")?.className).toBe("danger-action");
    await act(async () => button("Delete file")?.click());
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(container.textContent).toContain("The target changed or already exists");
    expect(mocks.invoke.mock.calls.filter(([command]) => command === "apply_remote_mutation")).toHaveLength(1);
  });

  it("provides previous and next navigation without reloading cached pages", async () => {
    mocks.invoke.mockResolvedValueOnce({ ...page(), hasMore: true });
    await act(async () => button("Refresh")?.click());
    expect(button("Previous page")?.disabled).toBe(true);
    mocks.invoke.mockResolvedValueOnce({ ...page(), pageSequence: 1, entries: [{ ...page().entries[2], name: "second.txt" }] });
    await act(async () => button("Next page")?.click());
    expect(container.textContent).toContain("Page 2");
    await act(async () => button("Previous page")?.click());
    expect(container.textContent).toContain("Page 1");
    expect(container.textContent).toContain("notes.txt");
    expect(container.textContent).not.toContain("second.txt");
  });

  it("moves row selection with arrow keys", async () => {
    const first = button("folder");
    await act(async () => {
      first?.focus();
      first?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    });
    expect(document.activeElement?.textContent).toBe("notes.txt");
    expect(container.querySelector("tr.selected")?.textContent).toContain("notes.txt");
  });

  it("attaches to its SSH terminal, filters hidden entries, and enters remote directories", async () => {
    expect(mocks.invoke).toHaveBeenCalledWith("attach_sftp", { terminal });
    expect(mocks.invoke).toHaveBeenCalledWith("list_remote_directory", { terminal, session: attached.sessionId, path: attached.path });
    expect(container.textContent).toContain("folder");
    expect(container.textContent).toContain("notes.txt");
    expect(container.textContent).not.toContain(".env");

    const hidden = [...container.querySelectorAll("button")].find((button) => button.textContent === "Hidden files");
    await act(async () => { hidden?.click(); });
    expect(container.textContent).toContain(".env");

    const folder = [...container.querySelectorAll<HTMLButtonElement>(".sftp-entry-name")].find((button) => button.textContent === "folder");
    await act(async () => { folder?.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); });
    expect(mocks.invoke).toHaveBeenCalledWith("list_remote_directory", { terminal, session: attached.sessionId, path: "/home/test/folder" });
  });

  it("creates entries through a form and confirms deletion with host, path, and type context", async () => {
    const newDirectory = [...container.querySelectorAll("button")].find((button) => button.textContent === "New directory");
    await act(async () => { newDirectory?.click(); });
    const name = document.body.querySelector<HTMLInputElement>('input[aria-label="Name"]');
    expect(name).not.toBeNull();
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(name, "reports");
      name?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const create = [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Confirm");
    await act(async () => { create?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    expect(mocks.invoke).toHaveBeenCalledWith("create_remote_entry", {
      terminal, session: attached.sessionId, parent: "/home/test", name: "reports", directory: true,
    });

    const file = [...container.querySelectorAll<HTMLButtonElement>(".sftp-entry-name")].find((button) => button.textContent === "notes.txt");
    await act(async () => { file?.click(); });
    const remove = [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Delete");
    await act(async () => { remove?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    expect(document.body.textContent).toContain("Build server");
    expect(document.body.textContent).toContain("/home/test/notes.txt");
    expect(document.body.textContent).toContain("File");
    expect(mocks.invoke).toHaveBeenCalledWith("prepare_remote_mutation", {
      terminal, session: attached.sessionId, entry: page().entries[2].id, mutation: { kind: "delete" },
    });
    const confirm = [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Delete file");
    await act(async () => { confirm?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    expect(mocks.invoke).toHaveBeenCalledWith("apply_remote_mutation", {
      terminal, session: attached.sessionId, token: "confirmation-token",
    });
  });

  it("reports dock, size, and collapse changes to the owning terminal pane", async () => {
    await act(async () => button("Move panel to the bottom")?.click());
    expect(onLayoutChange).toHaveBeenCalledWith({ ...layout, dock: "bottom" });
    const splitter = container.querySelector<HTMLElement>('[role="separator"][aria-label="Panel size"]');
    expect(splitter?.getAttribute("aria-valuenow")).toBe("36");
    await act(async () => splitter?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })));
    expect(onLayoutChange).toHaveBeenCalledWith({ ...layout, size: 38 });
    await act(async () => splitter?.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true })));
    expect(onLayoutChange).toHaveBeenCalledWith({ ...layout, size: 60 });
    await act(async () => button("Collapse remote files")?.click());
    expect(onLayoutChange).toHaveBeenCalledWith({ ...layout, collapsed: true });
  });

  it("keeps three transfer keys visible and the rest in one labeled menu", async () => {
    const toolbar = container.querySelector(".sftp-mutation-toolbar");
    const visible = [...(toolbar?.querySelectorAll<HTMLButtonElement>(":scope > button") ?? [])].map((entry) => entry.textContent);
    expect(visible).toEqual(["Upload file", "Download file", "New directory", "More file actions"]);
    const menu = toolbar?.querySelector<HTMLElement>("[role='menu']");
    expect(menu?.getAttribute("popover")).toBe("auto");
    expect([...(menu?.querySelectorAll("[role='menuitem'], [role='menuitemcheckbox']") ?? [])].map((item) => item.textContent)).toContain("Permissions");
  });

  it("deletes and renames the focused row from the keyboard", async () => {
    const notes = button("notes.txt");
    await act(async () => notes?.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true })));
    expect(mocks.invoke).toHaveBeenCalledWith("prepare_remote_mutation", {
      terminal, session: attached.sessionId, entry: page().entries[2].id, mutation: { kind: "delete" },
    });
    const confirmation = document.querySelector('[role="alertdialog"]');
    expect(confirmation?.textContent).toContain("/home/test/notes.txt");
    expect(confirmation?.querySelector("h2")?.textContent).toMatch(/^Delete notes\.txt from /u);
    expect(document.activeElement?.textContent).toBe("Cancel");
    await act(async () => button("Cancel")?.click());
    await act(async () => notes?.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true })));
    expect(document.body.querySelector<HTMLInputElement>('input[aria-label="Name"]')?.value).toBe("notes.txt");
  });

  it("names the production host in a file delete and in the panel header", async () => {
    await act(async () => root.render(<StrictMode><SftpPanel terminal={terminal} locale="en" hostLabel="prod-api" theme="graphite" layout={layout} production onLayoutChange={onLayoutChange} /></StrictMode>));
    expect(container.querySelector(".sftp-panel-header .prod-chip")?.textContent).toBe("Production");
    await act(async () => button("notes.txt")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true })));
    const confirmation = document.querySelector('[role="alertdialog"]');
    expect(confirmation?.getAttribute("data-tone")).toBe("danger");
    expect(confirmation?.querySelector("h2")?.textContent).toBe("Delete notes.txt from prod-api?");
    expect(confirmation?.querySelector(".confirm-target .prod-chip")?.textContent).toBe("Production");
    // A file delete does not carry the empty-directory note.
    expect(confirmation?.textContent).not.toContain("Directories must be empty");
  });

  it("opens a context menu with the clicked row's own actions", async () => {
    const show = vi.fn(), hide = vi.fn();
    Object.assign(HTMLElement.prototype, { showPopover: show, hidePopover: hide });
    try {
      const row = button("notes.txt")?.closest("tr");
      await act(async () => row?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 40, clientY: 40 })));
      expect(show).toHaveBeenCalled();
      const items = [...container.querySelectorAll(".sftp-menu [role='menuitem']")].map((item) => item.textContent);
      expect(items.slice(0, 2)).toEqual(["Edit", "Download file"]);
      expect(items).toContain("Delete");
      expect(items).not.toContain("Upload folder");
      expect(row?.getAttribute("aria-selected")).toBe("true");
    } finally {
      for (const key of ["showPopover", "hidePopover"]) Reflect.deleteProperty(HTMLElement.prototype, key);
    }
  });

  it("says when the SSH session behind the panel has ended", async () => {
    await act(async () => root.render(<StrictMode><SftpPanel terminal={terminal} locale="en" hostLabel="Build server" theme="graphite" layout={layout} ended onLayoutChange={onLayoutChange} /></StrictMode>));
    expect(container.querySelector(".sftp-ended")?.textContent).toBe("The SSH session ended. Reconnect to browse files again.");
  });

  it("keeps a file's extension visible when its name truncates", () => {
    expect(splitName("build-2026-10-22.tar.gz")).toEqual(["build-2026-10-22", ".tar.gz"]);
    expect(splitName("notes.txt")).toEqual(["notes", ".txt"]);
    expect(splitName(".env")).toEqual([".env", ""]);
    expect(splitName("Makefile")).toEqual(["Makefile", ""]);
  });
});
