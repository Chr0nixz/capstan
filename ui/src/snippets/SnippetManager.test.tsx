import { act, StrictMode, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { copy } from "../i18n";
import { SnippetManager } from "./SnippetManager";
import type { Snippet } from "./contracts";
import { decodeSnippetGroup } from "./contracts";

Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });

const snippets = [
  { id: "11111111-1111-4111-8111-111111111111", group_id: null, name: "Greeting", template: "echo {{name:default}}", auto_enter: true },
  { id: "22222222-2222-4222-8222-222222222222", group_id: null, name: "Show status", template: "echo ready", auto_enter: false },
  { id: "33333333-3333-4333-8333-333333333333", group_id: null, name: "Change directory", template: "cd {{目录:/tmp}}", auto_enter: true },
] as Snippet[];

let container: HTMLDivElement, root: Root;
const onRun = vi.fn(async (_input: string, _target: "current" | "broadcast") => undefined);
const onSaveSnippet = vi.fn(async (_snippet: Snippet): Promise<void> => undefined);

function managerProps(): ComponentProps<typeof SnippetManager> {
  return {
    labels: copy.en, groups: [], snippets, search: "", onSearch: () => undefined,
    onSaveGroup: async () => undefined, onDeleteGroup: async () => undefined,
    onSaveSnippet, onDeleteSnippet: async () => undefined, onRun,
    broadcastTargetsAvailable: true, onRequestConsumed: () => undefined, onClose: () => undefined,
  };
}

beforeEach(() => {
  onRun.mockReset();
  onRun.mockResolvedValue(undefined);
  onSaveSnippet.mockReset();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => root.render(<SnippetManager {...managerProps()} />));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

// The editor and variable form are portaled dialogs, so search the whole document.
function buttonByText(text: string): HTMLButtonElement | undefined {
  return [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === text || button.textContent?.startsWith(text));
}

describe("snippet manager run flow", () => {
  it("accepts one save while pending and preserves the draft after a rejected save", async () => {
    let rejectSave!: (error: unknown) => void;
    onSaveSnippet.mockImplementationOnce(() => new Promise<void>((_resolve, reject) => { rejectSave = reject; }));
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Edit snippet: Change directory']")!.click());
    const editor = document.body.querySelector<HTMLFormElement>(".snippet-editor")!;
    await act(async () => {
      editor.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      editor.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    expect(onSaveSnippet).toHaveBeenCalledTimes(1);
    expect(container.querySelector<HTMLButtonElement>("button[aria-label='New snippet']")?.disabled).toBe(true);
    expect(editor.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(true);
    await act(async () => rejectSave({ code: "vault_conflict" }));
    expect(editor.querySelector("[role='alert']")?.textContent).toBe(copy.en.snippetConflict);
    expect(editor.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe(snippets[2].template);
    expect(editor.querySelector<HTMLButtonElement>("button[type='submit']")?.disabled).toBe(false);
    onSaveSnippet.mockResolvedValueOnce(undefined);
    await act(async () => editor.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSaveSnippet).toHaveBeenCalledTimes(2);
    expect(document.body.querySelector(".snippet-editor")).toBeNull();
  });

  it("serializes group creation with other mutations and releases the guard after deletion fails", async () => {
    const group = decodeSnippetGroup({ id: "44444444-4444-4444-8444-444444444444", name: "Operations" });
    let finishSave!: () => void, rejectDelete!: (error: unknown) => void;
    const onSaveGroup = vi.fn(() => new Promise<void>((resolve) => { finishSave = resolve; }));
    const onDeleteGroup = vi.fn(() => new Promise<void>((_resolve, reject) => { rejectDelete = reject; }));
    const onDeleteSnippet = vi.fn(async () => undefined);
    await act(async () => root.render(<SnippetManager {...managerProps()} groups={[group]} onSaveGroup={onSaveGroup} onDeleteGroup={onDeleteGroup} onDeleteSnippet={onDeleteSnippet} />));
    const input = container.querySelector<HTMLInputElement>(".snippet-group-create input")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "新分组");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const create = container.querySelector<HTMLFormElement>(".snippet-group-create")!;
    await act(async () => {
      create.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      create.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
      container.querySelector<HTMLButtonElement>("button[aria-label='Delete snippet: Show status']")!.click();
    });
    expect(onSaveGroup).toHaveBeenCalledTimes(1);
    expect(onDeleteSnippet).not.toHaveBeenCalled();
    expect(input.disabled).toBe(true);
    await act(async () => finishSave());
    expect(input.value).toBe("");
    expect(input.disabled).toBe(false);
    const remove = container.querySelector<HTMLButtonElement>("button[aria-label='Delete group: Operations']")!;
    await act(async () => { remove.click(); remove.click(); });
    expect(onDeleteGroup).toHaveBeenCalledExactlyOnceWith(group.id);
    await act(async () => rejectDelete({ code: "group_in_use" }));
    expect(container.querySelector("[role='alert']")?.textContent).toBe(copy.en.snippetGroupInUse);
    expect(remove.disabled).toBe(false);
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Delete snippet: Show status']")!.click());
    expect(onDeleteSnippet).toHaveBeenCalledExactlyOnceWith(snippets[1].id);
  });

  it("retains edited group names across a refresh while updating untouched names", async () => {
    const first = decodeSnippetGroup({ id: "44444444-4444-4444-8444-444444444444", name: "Operations" });
    const second = decodeSnippetGroup({ id: "55555555-5555-4555-8555-555555555555", name: "Build" });
    await act(async () => root.render(<SnippetManager {...managerProps()} groups={[first, second]} />));
    const input = container.querySelector<HTMLInputElement>(".snippet-group-list input")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "本地草稿");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => root.render(<SnippetManager {...managerProps()} groups={[{ ...first, name: "Remote edit" }, { ...second, name: "Refreshed build" }]} />));
    expect([...container.querySelectorAll<HTMLInputElement>(".snippet-group-list input")].map((field) => field.value)).toEqual(["本地草稿", "Refreshed build"]);
  });

  it.each(["en", "zh-CN"] as const)("reports storage failure in %s without backend text or terminal connection errors", async (locale) => {
    onSaveSnippet.mockRejectedValueOnce(new Error("private backend path and diagnostic"));
    await act(async () => root.render(<SnippetManager {...managerProps()} labels={copy[locale]} />));
    await act(async () => container.querySelector<HTMLButtonElement>(`button[aria-label='${copy[locale].editSnippet}: Show status']`)!.click());
    await act(async () => document.body.querySelector<HTMLFormElement>(".snippet-editor")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    const alert = document.body.querySelector(".snippet-editor [role='alert']")?.textContent;
    expect(alert).toBe(copy[locale].snippetStorageFailed);
    expect(alert).not.toContain("private backend");
    expect(alert).not.toBe(copy[locale].connectionFailed);
    expect(onSaveSnippet).toHaveBeenCalledTimes(1);
  });

  it.each(["中".repeat(43), "bad\u0001name"])("validates group names before saving: %j", async (name) => {
    const onSaveGroup = vi.fn(async () => undefined);
    await act(async () => root.render(<SnippetManager {...managerProps()} onSaveGroup={onSaveGroup} />));
    const input = container.querySelector<HTMLInputElement>(".snippet-group-create input")!;
    const enterName = (value: string) => act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await enterName(name);
    await act(async () => container.querySelector<HTMLFormElement>(".snippet-group-create")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSaveGroup).not.toHaveBeenCalled();
    expect(container.querySelector("[role='alert']")?.textContent).toBe(copy.en.snippetGroupInvalid);
    await enterName("中".repeat(42));
    await act(async () => container.querySelector<HTMLFormElement>(".snippet-group-create")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSaveGroup).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ name: "中".repeat(42) }));
  });

  it.each([
    ["input", "中".repeat(86)],
    ["input", "bad\u0001name"],
    ["textarea", "中".repeat(2731)],
  ])("validates %s UTF-8 length and characters before saving", async (selector, value) => {
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Edit snippet: Change directory']")!.click());
    const editor = document.body.querySelector<HTMLFormElement>(".snippet-editor")!;
    const field = editor.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector)!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(selector === "input" ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype, "value")!.set!.call(field, value);
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => editor.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSaveSnippet).not.toHaveBeenCalled();
    expect(editor.querySelector("[role='alert']")?.textContent).toBe(copy.en.snippetInvalid);
    expect(field.value).toBe(value);
  });

  it("consumes a request synchronously once during effect replay and accepts a fresh request for the same snippet", async () => {
    const onRequestConsumed = vi.fn();
    const renderRequest = (requestId: string): void => root.render(<StrictMode><SnippetManager {...managerProps()}
      runRequest={{ requestId, snippetId: snippets[1].id }} onRequestConsumed={onRequestConsumed} /></StrictMode>);
    await act(async () => renderRequest("first-run"));
    expect(onRequestConsumed).toHaveBeenCalledExactlyOnceWith("first-run");
    expect(onRun).toHaveBeenCalledExactlyOnceWith("echo ready", "current");
    await act(async () => renderRequest("first-run"));
    expect(onRun).toHaveBeenCalledTimes(1);
    await act(async () => renderRequest("second-run"));
    expect(onRun).toHaveBeenCalledTimes(2);
    expect(onRequestConsumed).toHaveBeenLastCalledWith("second-run");
  });

  it("does not enqueue another run while its send is pending and handles a rejected send", async () => {
    let rejectSend!: (reason: Error) => void;
    onRun.mockImplementationOnce(() => new Promise<undefined>((_resolve, reject) => { rejectSend = reject; }));
    const run = buttonByText("Show status")!;
    await act(async () => { run.click(); run.click(); });
    expect(onRun).toHaveBeenCalledTimes(1);
    expect(run.disabled).toBe(true);
    await act(async () => rejectSend(new Error("backend")));
    expect(container.querySelector("[role='alert']")?.textContent).toBe(copy.en.operationFailed);
    expect(run.disabled).toBe(false);
    expect(onRun).toHaveBeenCalledTimes(1);
  });

  it("captures an edited group name before React applies the state update", async () => {
    const group = decodeSnippetGroup({ id: "44444444-4444-4444-8444-444444444444", name: "Operations" });
    const onSaveGroup = vi.fn(async () => undefined);
    await act(async () => root.render(<SnippetManager {...managerProps()} groups={[group]} onSaveGroup={onSaveGroup} />));
    const input = container.querySelector<HTMLInputElement>(".snippet-group-list input")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "运维");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(input.value).toBe("运维");
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Save group: Operations']")!.click());
    expect(onSaveGroup).toHaveBeenCalledExactlyOnceWith({ ...group, name: "运维" });
  });

  it("saves a Chinese variable template and runs its default or edited value, with cancellation sending nothing", async () => {
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Edit snippet: Change directory']")!.click());
    const editor = document.body.querySelector<HTMLFormElement>(".snippet-editor")!;
    expect(editor.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("cd {{目录:/tmp}}");
    await act(async () => editor.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSaveSnippet).toHaveBeenCalledExactlyOnceWith(snippets[2]);

    await act(async () => buttonByText("Change directory")!.click());
    let variable = document.body.querySelector<HTMLInputElement>(".snippet-variable-form input")!;
    expect(variable.value).toBe("/tmp");
    expect(variable.closest("label")?.textContent).toContain("目录");
    await act(async () => buttonByText("Cancel")!.click());
    expect(onRun).not.toHaveBeenCalled();
    await act(async () => buttonByText("Change directory")!.click());
    await act(async () => buttonByText("Send to terminal")!.click());
    expect(onRun).toHaveBeenCalledExactlyOnceWith("cd /tmp\r", "current");
    await act(async () => buttonByText("Change directory")!.click());
    variable = document.body.querySelector<HTMLInputElement>(".snippet-variable-form input")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(variable, "/srv/项目");
      variable.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => buttonByText("Send to terminal")!.click());
    expect(onRun).toHaveBeenLastCalledWith("cd /srv/项目\r", "current");
    expect(onSaveSnippet).toHaveBeenCalledTimes(1);
    expect(snippets[2].template).toBe("cd {{目录:/tmp}}");
  });

  it("saves and runs a template whose Go-template braces are not variables", async () => {
    const template = "docker ps --format '{{.Names}}' --filter name={{名称:web}}";
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Edit snippet: Change directory']")!.click());
    const editor = document.body.querySelector<HTMLFormElement>(".snippet-editor")!;
    const field = editor.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(field, template);
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => editor.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(onSaveSnippet).toHaveBeenCalledExactlyOnceWith({ ...snippets[2], template });

    await act(async () => root.render(<SnippetManager {...managerProps()} snippets={[{ ...snippets[2], template }]} />));
    await act(async () => buttonByText("Change directory")!.click());
    const variables = document.body.querySelectorAll<HTMLInputElement>(".snippet-variable-form input");
    expect([...variables].map((input) => input.value)).toEqual(["web"]);
    await act(async () => buttonByText("Send to terminal")!.click());
    expect(onRun).toHaveBeenCalledExactlyOnceWith("docker ps --format '{{.Names}}' --filter name=web\r", "current");
  });

  it("uses variable defaults, cancels without sending, honors the selected target and auto-enter", async () => {
    const target = container.querySelector<HTMLSelectElement>(".snippet-target select");
    await act(async () => {
      if (target) {
        const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set;
        setter?.call(target, "broadcast");
        target.dispatchEvent(new Event("change", { bubbles: true }));
      }
      buttonByText("Greeting")?.click();
    });
    let variable = document.body.querySelector<HTMLInputElement>(".snippet-variable-form input");
    expect(variable?.value).toBe("default");
    await act(async () => {
      buttonByText("Cancel")?.click();
    });
    expect(onRun).not.toHaveBeenCalled();

    await act(async () => { buttonByText("Greeting")?.click(); });
    variable = document.body.querySelector<HTMLInputElement>(".snippet-variable-form input");
    expect(variable?.value).toBe("default");
    await act(async () => { buttonByText("Send to terminal")?.click(); });
    expect(onRun).toHaveBeenCalledExactlyOnceWith("echo default\r", "broadcast");

    await act(async () => { buttonByText("Show status")?.click(); });
    expect(onRun).toHaveBeenLastCalledWith("echo ready", "broadcast");
  });
});
