import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { useWorkspace } from "./workspace";
import { paneLeaves } from "./panes/tree";
import { paneInputs } from "./panes/input";
import { copy } from "./i18n";
import m4 from "../../docs/agent/fixtures/M4-contract.json";
import type { Snippet } from "./snippets/contracts";
import type { HostId, TerminalLaunch, TerminalSessionId, TerminalSnapshot } from "./terminal/contracts";

Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });

const lifecycle = vi.hoisted(() => ({ started: [] as TerminalLaunch[], stopped: [] as TerminalLaunch[] }));
vi.mock("./terminal/TerminalPane", async () => {
  const { useEffect } = await import("react");
  return { TerminalPane: ({ launch }: { launch: TerminalLaunch }) => {
    useEffect(() => { lifecycle.started.push(launch); return () => { lifecycle.stopped.push(launch); }; }, [launch]);
    return null;
  } };
});
vi.mock("./window/Titlebar", () => ({ Titlebar: () => null }));

const host = {
  id: "host-1", name: "Build Server", address: "build.example.test", port: 22, username: "deploy",
  credential_id: "credential-1", group_id: null, tags: [], favorite: false, revision: 0,
  working_directory: null, startup_command: null, encoding: "utf-8", theme: "graphite", agent_forwarding: false, production: false,
};
const group = { id: "group-1", name: "Platform", parent_id: null, production: false };
const groups = [group, { id: "group-2", name: "Build", parent_id: null, production: false }];
const defaultInvoke = async (command: string, _args?: unknown): Promise<unknown> => {
  if (command === "vault_status") return { exists: true, unlocked: true, revision: 3, epoch: 1 };
  if (command === "vault_hello_status") return { ...m4.helloStatus, enabled: false };
  if (command === "list_hosts") return [{ ...host }];
  if (command === "list_credentials") return [
    { id: "credential-1", name: "Deploy", revision: 0, kind: "password" },
    { id: "credential-agent", name: "Work agent", revision: 0, kind: "agent" },
  ];
  if (command === "list_groups") return groups.map((item) => ({ ...item }));
  if (command === "list_snippet_groups" || command === "list_snippets") return [];
  if (command === "remote_edit_snapshot") return { revision: "0", sessions: [], recoveryIncomplete: false };
  if (command === "list_trusted_host_keys") return [];
  if (command === "vault_preferences") return { auto_lock_minutes: 30, disconnect_on_lock: false };
  if (command === "list_terminal_themes") return [];
  if (command === "load_desktop_preferences") return m4.settingsDefaults;
  if (command === "list_launch_requests") return [];
  if (command === "quake_status") return { enabled: false, errorCode: null };
  if (command === "list_agent_identities") return [{ algorithm: "ssh-ed25519", fingerprint: "SHA256:example-key" }];
  if (command === "create_agent_credential") return "credential-agent";
  if (command === "preview_vault_backup") return {
    token: "short-lived-token",
    preview: {
      baseRevision: 3, hostCount: 2, groupCount: 1, credentialCount: 1, trustCount: 1, snippetGroupCount: 0, snippetCount: 0,
      duplicateHosts: [{ existingName: "Build Server", incomingName: "Imported build", target: "build.example.test", port: 22 }],
      trustConflicts: [],
    },
  };
  return 4;
};
const invoke = vi.fn(defaultInvoke);

let container: HTMLDivElement;
let root: Root;
let trustChallengeListener: ((event: { payload: unknown }) => void) | null;
let appEvents: Map<string, (event: { payload: unknown }) => void>;

beforeEach(async () => {
  invoke.mockReset();
  invoke.mockImplementation(defaultInvoke);
  container = document.createElement("div");
  trustChallengeListener = null;
  appEvents = new Map();
  document.body.append(container);
  root = createRoot(container);
  Object.defineProperty(window, "__TAURI__", {
    configurable: true,
    value: {
      core: {
        Channel: class { onmessage: ((message: unknown) => void) | null = null; },
        invoke,
      },
      event: { listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
        if (name === "ssh-trust-challenge") trustChallengeListener = handler;
        appEvents.set(name, handler);
        return () => undefined;
      } },
    },
  });
  await act(async () => {
    root.render(<App />);
    await new Promise((resolve) => window.setTimeout(resolve, 0));
  });
});

afterEach(async () => {
  vi.restoreAllMocks();
  await act(async () => root.unmount());
  container.remove();
  Reflect.deleteProperty(window, "__TAURI__");
});

function dragEvent(type: string, entries: Record<string, string>): Event {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "dataTransfer", {
    value: {
      effectAllowed: "",
      getData: (key: string) => entries[key] ?? "",
      setData: (key: string, value: string) => { entries[key] = value; },
    },
  });
  return event;
}

function bodyButton(text: string): HTMLButtonElement | undefined {
  return [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === text);
}

async function answerConfirmation(accept: string | null): Promise<string> {
  const dialog = document.body.querySelector<HTMLElement>("[role='alertdialog']");
  const text = dialog?.textContent ?? "";
  expect(dialog).not.toBeNull();
  expect(document.activeElement?.textContent).toBe("Cancel");
  await act(async () => {
    [...(dialog?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((button) => button.textContent === (accept ?? "Cancel"))?.click();
    await new Promise((resolve) => window.setTimeout(resolve, 0));
  });
  expect(document.body.querySelector("[role='alertdialog']")).toBeNull();
  return text;
}

async function openSettings(tab?: string): Promise<void> {
  await act(async () => { container.querySelector<HTMLButtonElement>("button[aria-label='Settings']")?.click(); });
  if (!tab) return;
  await act(async () => {
    [...container.querySelectorAll<HTMLButtonElement>(".segmented [role='tab']")].find((button) => button.textContent === tab)?.click();
  });
}

describe("App host workflows", () => {
  it("enables and disables Hello with native-only authentication arguments", async () => {
    let enabled = false;
    invoke.mockImplementation(async (command, args) => {
      if (command === "vault_hello_status") return { ...m4.helloStatus, enabled };
      if (command === "enable_vault_hello") { enabled = true; return; }
      if (command === "disable_vault_hello") { enabled = false; return; }
      return defaultInvoke(command, args);
    });
    await openSettings();
    expect(container.textContent).toContain(copy.en.hello.hint);
    await act(async () => { bodyButton(copy.en.hello.enable)!.click(); });
    expect(invoke).toHaveBeenCalledWith("enable_vault_hello", { locale: "en" });
    expect(bodyButton(copy.en.hello.disable)).toBeDefined();
    await act(async () => { bodyButton(copy.en.hello.disable)!.click(); });
    expect(invoke).toHaveBeenCalledWith("disable_vault_hello", undefined);
    expect(bodyButton(copy.en.hello.enable)).toBeDefined();
  });

  it.each(m4.errorCodes.hello)("keeps master unlock available after %s", async (code) => {
    invoke.mockImplementation(async (command, args) => {
      if (command === "vault_status") return { exists: true, unlocked: false, revision: null, epoch: 2 };
      if (command === "vault_hello_status") return m4.helloStatus;
      if (command === "unlock_vault_hello") throw { code };
      return defaultInvoke(command, args);
    });
    await act(async () => { appEvents.get("vault-auto-locked")?.({ payload: null }); });
    await act(async () => { bodyButton(copy.en.hello.unlock)!.click(); });
    expect(bodyButton(copy.en.unlockVault)?.disabled).toBe(false);
    expect(container.querySelector(".host-row")).toBeNull();
    expect(container.querySelector("[role='alert']")).not.toBeNull();
    expect(invoke).not.toHaveBeenCalledWith("unlock_vault", expect.anything());
    await act(async () => { bodyButton(copy.en.unlockVault)!.click(); });
    expect(invoke).toHaveBeenCalledWith("unlock_vault", { locale: "en" });
  });

  it("blocks duplicate prompts and refreshes private views only after Hello succeeds", async () => {
    let unlocked = false;
    let finish: (() => void) | undefined;
    invoke.mockImplementation(async (command, args) => {
      if (command === "vault_status") return { exists: true, unlocked, revision: unlocked ? 3 : null, epoch: 2 };
      if (command === "vault_hello_status") return m4.helloStatus;
      if (command === "unlock_vault_hello") { await new Promise<void>((resolve) => { finish = resolve; }); unlocked = true; return; }
      return defaultInvoke(command, args);
    });
    await act(async () => { appEvents.get("vault-auto-locked")?.({ payload: null }); });
    invoke.mockClear();
    await act(async () => { bodyButton(copy.en.hello.unlock)!.click(); });
    expect(bodyButton(copy.en.hello.unlock)?.disabled).toBe(true);
    expect(bodyButton(copy.en.unlockVault)?.disabled).toBe(true);
    expect(invoke).not.toHaveBeenCalledWith("list_hosts", undefined);
    await act(async () => { bodyButton(copy.en.hello.unlock)!.click(); finish!(); });
    expect(invoke.mock.calls.filter(([name]) => name === "unlock_vault_hello")).toHaveLength(1);
    expect(container.querySelector(".host-row")).not.toBeNull();
  });

  it("reports invalidation after password unlock and allows master unlock when Hello is unavailable", async () => {
    let unlocked = false;
    invoke.mockImplementation(async (command, args) => {
      if (command === "vault_status") return { exists: true, unlocked, revision: unlocked ? 3 : null, epoch: 2 };
      if (command === "vault_hello_status") return { ...m4.helloStatus, available: false, invalidated: unlocked };
      if (command === "unlock_vault") { unlocked = true; return; }
      return defaultInvoke(command, args);
    });
    await act(async () => { appEvents.get("vault-auto-locked")?.({ payload: null }); });
    expect(bodyButton(copy.en.hello.unlock)?.disabled).toBe(true);
    expect(bodyButton(copy.en.unlockVault)?.disabled).toBe(false);
    expect(container.textContent).not.toContain(copy.en.hello.invalidated);
    await act(async () => { bodyButton(copy.en.unlockVault)!.click(); });
    expect(container.textContent).toContain(copy.en.hello.invalidated);
  });
  const launch = async (kind: "ssh" | "host" | "local", details: unknown, matches: string[] = []): Promise<string> => {
    const requestId = crypto.randomUUID();
    const fallback = invoke.getMockImplementation() ?? defaultInvoke;
    invoke.mockImplementation(async (command, args) => command === "inspect_launch_request" ? details
      : command === "match_launch_hosts" ? matches : fallback(command, args));
    await act(async () => { appEvents.get("launch-request")?.({ payload: { requestId, kind, display: kind === "local" ? "" : "user@host:22" } }); });
    return requestId;
  };

  it("confirms an SSH link before opening the exact matching saved host", async () => {
    await act(async () => useWorkspace.getState().addTerminal());
    const local = useWorkspace.getState().activeTab;
    // Rust normalizes the address; the UI must trust its IDs, not compare strings itself.
    const linked = `${host.address.toUpperCase()}.`;
    const requestId = await launch("ssh", { kind: "ssh", target: { username: host.username, host: linked, port: host.port } }, [host.id]);
    expect(useWorkspace.getState().activeTab).toBe(local);
    const text = await answerConfirmation(copy.en.sshLinkContinue);
    expect(invoke).toHaveBeenCalledWith("match_launch_hosts", { requestId });
    expect(text).toContain(linked);
    expect(text).toContain(host.username);
    expect(useWorkspace.getState().tabs.find((tab) => tab.id === useWorkspace.getState().activeTab)?.launch).toMatchObject({ kind: "ssh", host_id: host.id });
    expect(invoke).toHaveBeenCalledWith("dismiss_launch_request", { requestId });
  });

  it("opens an unknown SSH target as a draft and requires a saved credential before connecting", async () => {
    await act(async () => useWorkspace.getState().addTerminal());
    const local = useWorkspace.getState().activeTab;
    const requestId = await launch("ssh", { kind: "ssh", target: { username: "new-user", host: "new.example.test", port: 2222 } });
    await answerConfirmation(copy.en.sshLinkContinue);
    expect(useWorkspace.getState().activeTab).toBe(local);
    const inputs = [...container.querySelectorAll<HTMLInputElement>(".host-editor input")];
    expect(inputs.some((input) => input.value === "new.example.test")).toBe(true);
    expect(inputs.some((input) => input.value === "new-user")).toBe(true);
    await act(async () => container.querySelector<HTMLFormElement>(".host-editor")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(container.querySelector("[aria-invalid='true']")).not.toBeNull();
    expect(invoke.mock.calls.some(([command]) => command === "save_host")).toBe(false);
    await act(async () => container.querySelector<HTMLButtonElement>(".host-editor button[aria-label='Close panel']")!.click());
    expect(invoke).toHaveBeenCalledWith("dismiss_launch_request", { requestId });
  });

  it("discards a saved-host request when native unlock is canceled", async () => {
    invoke.mockImplementation(async (command, args) => {
      if (command === "vault_status") return { exists: true, unlocked: false, revision: null, epoch: 1 };
      if (command === "unlock_vault") throw { code: "canceled" };
      return defaultInvoke(command, args);
    });
    const before = useWorkspace.getState().activeTab;
    const requestId = await launch("host", { kind: "host", hostId: host.id });
    expect(invoke).toHaveBeenCalledWith("unlock_vault", { locale: "en" });
    expect(invoke).toHaveBeenCalledWith("dismiss_launch_request", { requestId });
    expect(useWorkspace.getState().activeTab).toBe(before);
    expect(document.body.querySelector("[role='alertdialog']")).toBeNull();
  });

  it("offers a choice when multiple saved hosts match and opens only the chosen host", async () => {
    invoke.mockImplementation(async (command, args) => {
      if (command === "list_hosts") return [{ ...host }, { ...host, id: "matching-host-2", name: "Other account profile" }];
      return defaultInvoke(command, args);
    });
    await act(async () => useWorkspace.getState().addTerminal());
    const local = useWorkspace.getState().activeTab;
    const requestId = await launch("ssh", { kind: "ssh", target: { username: host.username, host: host.address, port: 22 } }, [host.id, "matching-host-2"]);
    await answerConfirmation(copy.en.sshLinkContinue);
    expect(useWorkspace.getState().activeTab).toBe(local);
    const choices = [...document.body.querySelectorAll<HTMLButtonElement>(".launch-choice-list button")];
    expect(choices).toHaveLength(2);
    await act(async () => choices[1].click());
    expect(useWorkspace.getState().tabs.find((tab) => tab.id === useWorkspace.getState().activeTab)?.launch).toMatchObject({ kind: "ssh", host_id: "matching-host-2" });
    expect(invoke).toHaveBeenCalledWith("dismiss_launch_request", { requestId });
  });

  it("connects an unknown SSH target only after its host form is saved with a credential", async () => {
    await act(async () => useWorkspace.getState().addTerminal());
    const local = useWorkspace.getState().activeTab;
    const requestId = await launch("ssh", { kind: "ssh", target: { username: "new-user", host: "new.example.test", port: 2222 } });
    await answerConfirmation(copy.en.sshLinkContinue);
    expect(useWorkspace.getState().activeTab).toBe(local);
    await act(async () => {
      const select = [...container.querySelectorAll<HTMLSelectElement>(".host-editor select")].find((input) => [...input.options].some((option) => option.value === "credential-1"))!;
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, "credential-1");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => container.querySelector<HTMLFormElement>(".host-editor")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    const saved = invoke.mock.calls.find(([command]) => command === "save_host")?.[1] as { host: { id: HostId; credential_id: string; address: string } };
    expect(saved.host).toMatchObject({ credential_id: "credential-1", address: "new.example.test" });
    expect(useWorkspace.getState().tabs.find((tab) => tab.id === useWorkspace.getState().activeTab)?.launch).toMatchObject({ kind: "ssh", host_id: saved.host.id });
    expect(invoke).toHaveBeenCalledWith("dismiss_launch_request", { requestId });
  });

  it("requires confirmation for a taskbar host request and honors cancellation", async () => {
    await act(async () => useWorkspace.getState().addTerminal());
    const local = useWorkspace.getState().activeTab;
    const requestId = await launch("host", { kind: "host", hostId: host.id });
    expect(await answerConfirmation(null)).toContain(host.address);
    expect(useWorkspace.getState().activeTab).toBe(local);
    expect(invoke).toHaveBeenCalledWith("dismiss_launch_request", { requestId });
  });

  it("keeps recent hosts opt-in, explains the privacy boundary and exposes clear", async () => {
    await openSettings("Desktop");
    const recent = [...container.querySelectorAll<HTMLLabelElement>("label")].find((label) => label.textContent === copy.en.jumpListRecentHosts)!;
    expect(recent.querySelector<HTMLInputElement>("input")!.checked).toBe(false);
    expect(container.textContent).toContain(copy.en.jumpListPrivacy);
    await act(async () => bodyButton(copy.en.clearRecentHosts)!.click());
    expect(invoke).toHaveBeenCalledWith("clear_recent_hosts", undefined);
    expect(container.textContent).toContain(copy.en.recentHostsCleared);
  });

  it("opens local requests using only the opaque request ID", async () => {
    const requestId = await launch("local", { kind: "local" });
    expect(useWorkspace.getState().tabs.find((tab) => tab.id === useWorkspace.getState().activeTab)?.launch).toEqual({ kind: "local", options: { launch_request_id: requestId } });
  });

  it("previews a native theme before saving, then offers it to hosts and the command palette", async () => {
    invoke.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "preview_terminal_themes") return { token: m4.themePreviewToken, themes: [m4.terminalTheme] };
      if (command === "save_terminal_theme_preview") return [m4.terminalTheme];
      return defaultInvoke(command, args);
    });
    await openSettings("Desktop");
    const importTheme = (): Promise<void> => act(async () => { bodyButton(copy.en.importTheme)?.click(); });
    await importTheme();
    expect(container.querySelector(".theme-preview")?.textContent).toContain(m4.terminalTheme.name);
    expect(container.querySelectorAll(".theme-swatches span")).toHaveLength(16);
    expect(invoke).toHaveBeenCalledWith("preview_terminal_themes", { locale: "en" });
    expect(invoke.mock.calls.some(([command]) => command === "save_terminal_theme_preview")).toBe(false);
    await act(async () => { bodyButton(copy.en.cancelEdit)?.click(); });
    expect(invoke).toHaveBeenCalledWith("discard_terminal_theme_preview", { token: m4.themePreviewToken });
    await importTheme();
    await act(async () => { bodyButton(copy.en.saveImportedThemes(1))?.click(); });
    expect(invoke).toHaveBeenCalledWith("save_terminal_theme_preview", { token: m4.themePreviewToken });
    expect(container.querySelector(".theme-list")?.textContent).toContain(m4.terminalTheme.name);
    await act(async () => { container.querySelector<HTMLButtonElement>("button[aria-label='Close panel']")?.click(); });
    await act(async () => { container.querySelector<HTMLButtonElement>("button[aria-label='Add host']")?.click(); });
    expect([...container.querySelectorAll("select option")].find((option) => option.getAttribute("value") === m4.terminalTheme.id)?.textContent).toBe(m4.terminalTheme.name);
    await act(async () => { container.querySelector<HTMLButtonElement>("button[aria-label='Close panel']")?.click(); });
    await act(async () => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "p", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true })); });
    expect([...document.body.querySelectorAll(".command-result")].some((action) => action.textContent?.includes(copy.en.themeAction(m4.terminalTheme.name)))).toBe(true);
  });
  it("lists authenticated recovery revisions and requires an explicit restore choice", async () => {
    const recoveryCandidate = { token: "opaque-recovery-token", kind: "backup" as const, revision: 7 };
    invoke.mockImplementation(async (command: string) => {
      if (command === "vault_status") return { exists: true, unlocked: false, revision: null, epoch: 0 };
      if (command === "list_vault_recovery_candidates") return [recoveryCandidate];
      return 4;
    });
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => {
      root.render(<App />);
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });

    const find = [...container.querySelectorAll("button")].find((button) => button.textContent === "Find recovery files");
    expect(find).not.toBeUndefined();
    await act(async () => {
      find?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(container.textContent).toContain("Encrypted backup");
    expect(container.textContent).toContain("Revision 7");
    expect(invoke).toHaveBeenCalledWith("list_vault_recovery_candidates", { locale: "en" });

    const restore = [...container.querySelectorAll("button")].find((button) => button.textContent === "Restore this revision");
    expect(restore).not.toBeUndefined();
    await act(async () => {
      restore?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(await answerConfirmation(null)).toContain("revision 7");
    expect(invoke).not.toHaveBeenCalledWith("restore_vault_recovery_candidate", expect.anything());
    await act(async () => {
      restore?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    await answerConfirmation("Replace with revision 7");
    expect(invoke).toHaveBeenCalledWith("restore_vault_recovery_candidate", {
      token: "opaque-recovery-token", locale: "en",
    });
    const recoveryArgs = invoke.mock.calls
      .filter(([command]) => command.includes("recovery"))
      .map(([, args]) => JSON.stringify(args) ?? "");
    expect(recoveryArgs.every((args) => !args.includes("vault.capstan"))).toBe(true);
  });

  it("keeps the vault locked and explains that a failed recovery authentication changed nothing", async () => {
    invoke.mockImplementation(async (command: string): Promise<unknown> => {
      if (command === "vault_status") return { exists: true, unlocked: false, revision: null, epoch: 0 };
      if (command === "list_vault_recovery_candidates") return [{ token: "opaque-token", kind: "pending", revision: 6 }];
      if (command === "restore_vault_recovery_candidate") throw { code: "vault_authentication" };
      return 4;
    });
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => {
      root.render(<App />);
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    const find = [...container.querySelectorAll("button")].find((button) => button.textContent === "Find recovery files");
    await act(async () => { find?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    const restore = [...container.querySelectorAll("button")].find((button) => button.textContent === "Restore this revision");
    await act(async () => { restore?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    await answerConfirmation("Replace with revision 6");

    expect(container.textContent).toContain("That password did not authenticate this recovery file. The vault was not replaced.");
    expect(container.textContent).toContain("Unlock vault");
    expect(invoke).not.toHaveBeenCalledWith("vault_status", expect.anything());
  });

  it("reports the bounded recovery scan limit with actionable guidance", async () => {
    invoke.mockImplementation(async (command: string): Promise<unknown> => {
      if (command === "vault_status") return { exists: true, unlocked: false, revision: null, epoch: 0 };
      if (command === "list_vault_recovery_candidates") throw { code: "vault_recovery_limit" };
      return 4;
    });
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => {
      root.render(<App />);
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });

    const find = [...container.querySelectorAll("button")].find((button) => button.textContent === "Find recovery files");
    await act(async () => { find?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    expect(container.textContent).toContain("There are too many recovery files to check safely.");
    expect(container.textContent).toContain("Move older recovery files out of the vault directory");
  });

  it("enables and persists agent forwarding only for an agent credential", async () => {
    const edit = container.querySelector<HTMLButtonElement>(".host-edit");
    expect(edit).not.toBeNull();
    await act(async () => { edit?.click(); });
    const editor = container.querySelector<HTMLFormElement>("form.host-editor");
    expect(editor).not.toBeNull();
    expect(container.querySelector("[role='dialog']")?.textContent).toContain("Edit host");

    const credential = [...(editor?.querySelectorAll("select") ?? [])].find((select) =>
      select.closest("label")?.textContent?.includes("Choose a credential"),
    );
    expect(credential).not.toBeUndefined();
    await act(async () => {
      if (credential) {
        const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set;
        setter?.call(credential, "credential-agent");
        credential.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });

    const forwarding = [...(editor?.querySelectorAll<HTMLInputElement>('input[type="checkbox"]') ?? [])].find((input) =>
      input.closest("label")?.textContent?.includes("Allow this host to use the SSH agent"),
    );
    expect(forwarding?.disabled).toBe(false);
    await act(async () => { forwarding?.click(); });
    await act(async () => {
      editor?.querySelector<HTMLButtonElement>('button[type="submit"]')?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });

    expect(invoke).toHaveBeenCalledWith("save_host", expect.objectContaining({
      expectedRevision: 3,
      host: expect.objectContaining({
        id: "host-1", credential_id: "credential-agent", agent_forwarding: true,
      }),
    }));
  });

  it("shows redacted host trust and requires confirmation before forgetting it", async () => {
    const key = {
      target: "build.example.test", port: 2222, algorithm: "ssh-ed25519",
      fingerprint: "SHA256:trusted-key", first_trusted_at: 1_800_000_000,
    };
    invoke.mockImplementation(async (command: string): Promise<unknown> => {
      if (command === "list_trusted_host_keys") return [key];
      if (command === "vault_status") return { exists: true, unlocked: true, revision: 3, epoch: 1 };
      if (command === "list_hosts") return [{ ...host }];
      if (command === "list_credentials") return [{ id: "credential-1", name: "Deploy", revision: 0, kind: "password" }];
      if (command === "list_groups") return groups.map((item) => ({ ...item }));
      if (command === "vault_preferences") return { auto_lock_minutes: 30, disconnect_on_lock: false };
      return 4;
    });
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => {
      root.render(<App />);
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    await openSettings("Host keys");
    const section = container.querySelector(".trust-manager");
    expect(section?.textContent).toContain("Trusted SSH host keys");
    expect(section?.textContent).toContain("SHA256:trusted-key");
    expect(section?.textContent).not.toContain("public-key");
    const forget = [...(section?.querySelectorAll("button") ?? [])].find((button) => button.textContent === "Forget key");
    expect(section?.textContent).toContain("build.example.test:2222");
    await act(async () => { forget?.click(); });
    expect(await answerConfirmation(null)).toContain("SHA256:trusted-key");
    expect(invoke).not.toHaveBeenCalledWith("forget_trusted_host_key", expect.anything());
    await act(async () => { forget?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    await answerConfirmation("Forget build.example.test:2222");
    expect(invoke).toHaveBeenCalledWith("forget_trusted_host_key", {
      target: "build.example.test", port: 2222, fingerprint: "SHA256:trusted-key", expectedRevision: 3,
    });
  });

  it("shows old and new fingerprints for a changed host key without offering trust acceptance", async () => {
    expect(trustChallengeListener).not.toBeNull();
    await act(async () => {
      trustChallengeListener?.({ payload: {
        challengeId: "changed-notice", attemptId: "attempt-1", hostId: "host-1", hostRevision: 0,
        target: "build.example.test", port: 22, algorithm: "ssh-ed25519", publicKey: "new-public-key",
        fingerprint: "SHA256:new-key", knownAlgorithm: "ssh-ed25519", knownFingerprint: "SHA256:old-key",
        keyChanged: true,
      } });
    });
    const dialog = container.querySelector<HTMLDialogElement>("dialog.trust-dialog");
    expect(dialog?.textContent).toContain("SHA256:old-key");
    expect(dialog?.textContent).toContain("SHA256:new-key");
    expect([...(dialog?.querySelectorAll(".fp-changed") ?? [])].map((chunk) => chunk.textContent)).toEqual(["new-"]);
    expect(dialog?.querySelector(".verify-command code")?.textContent).toBe("ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub");
    expect(dialog?.textContent).toContain("Build Server (build.example.test:22)");
    const actions = [...(dialog?.querySelectorAll<HTMLButtonElement>(".dialog-actions button") ?? [])];
    expect(actions.map((button) => button.textContent)).toEqual(["Close and keep blocked", "Open host keys"]);
    expect(actions.some((button) => /trust/i.test(button.textContent ?? ""))).toBe(false);
    await act(async () => { actions[1]?.click(); });
    expect(invoke).not.toHaveBeenCalledWith("answer_host_trust", expect.anything());
    expect(container.querySelector(".trust-manager")).not.toBeNull();
  });

  it("moves a dragged host to the dropped group through the revisioned command", async () => {
    await openSettings("Groups");
    const row = container.querySelector(".host-row");
    const target = container.querySelector(".group-list li");
    expect(row).not.toBeNull();
    expect(target).not.toBeNull();
    const entries: Record<string, string> = {};
    await act(async () => {
      row?.dispatchEvent(dragEvent("dragstart", entries));
      target?.dispatchEvent(dragEvent("drop", entries));
      await Promise.resolve();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(invoke).toHaveBeenCalledWith("save_host", expect.objectContaining({
      expectedRevision: 3,
      host: expect.objectContaining({ id: "host-1", group_id: "group-1" }),
    }));
  });

  it("nests a dragged group under its drop target", async () => {
    await openSettings("Groups");
    const rows = container.querySelectorAll(".group-list li");
    expect(rows).toHaveLength(2);
    const entries: Record<string, string> = {};
    await act(async () => {
      rows[0]?.dispatchEvent(dragEvent("dragstart", entries));
      rows[1]?.dispatchEvent(dragEvent("drop", entries));
      await Promise.resolve();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(invoke).toHaveBeenCalledWith("save_group", expect.objectContaining({
      expectedRevision: 3,
      group: expect.objectContaining({ id: "group-1", parent_id: "group-2" }),
    }));
  });

  it("persists the auto-lock and disconnect preferences", async () => {
    await openSettings();
    const timeout = [...container.querySelectorAll("input")].find((input) => input.type === "number");
    const disconnect = [...container.querySelectorAll("input")].find((input) => input.type === "checkbox");
    const save = [...container.querySelectorAll("button")].find((button) => button.textContent?.includes("Save security settings"));
    expect(timeout).not.toBeUndefined();
    expect(disconnect).not.toBeUndefined();
    expect(save).not.toBeUndefined();
    await act(async () => {
      if (timeout) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setter?.call(timeout, "45");
        timeout.dispatchEvent(new Event("input", { bubbles: true }));
        timeout.dispatchEvent(new Event("change", { bubbles: true }));
      }
      disconnect?.click();
      save?.click();
      await Promise.resolve();
    });
    expect(invoke).toHaveBeenCalledWith("update_vault_preferences", {
      autoLockMinutes: 45,
      disconnectOnLock: true,
      expectedRevision: 3,
    });
  });

  it("loads public agent identity metadata and stores only its selected fingerprint", async () => {
    await act(async () => { container.querySelector<HTMLButtonElement>("button[aria-label='Add host']")?.click(); });
    const name = container.querySelector<HTMLInputElement>("input[aria-label='Credential name']");
    expect(name).not.toBeNull();
    await act(async () => {
      if (name) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setter?.call(name, "Work agent");
        name.dispatchEvent(new Event("input", { bubbles: true }));
        name.dispatchEvent(new Event("change", { bubbles: true }));
      }
    });
    const load = [...container.querySelectorAll("button")].find((button) => button.textContent?.includes("Load agent identities"));
    expect(load).not.toBeUndefined();
    await act(async () => {
      load?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    const add = [...container.querySelectorAll("button")].find((button) => button.textContent?.includes("Add agent credential"));
    expect(add).not.toBeUndefined();
    await act(async () => {
      add?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(invoke).toHaveBeenCalledWith("create_agent_credential", {
      name: "Work agent", agent: "open_ssh", fingerprint: "SHA256:example-key",
    });
  });

  it("manages credential metadata without placing secrets in the UI", async () => {
    await openSettings("Credentials");
    const section = container.querySelector(".credential-manager");
    expect(section).not.toBeNull();
    expect(section?.textContent).toContain("Deploy");
    expect(section?.textContent).toContain("Password");

    const rename = [...(section?.querySelectorAll("button") ?? [])].find((button) => button.textContent === "Rename");
    await act(async () => { rename?.click(); });
    const nameInput = section?.querySelector<HTMLInputElement>("input");
    expect(nameInput).not.toBeNull();
    await act(async () => {
      if (nameInput) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setter?.call(nameInput, "Production deploy");
        nameInput.dispatchEvent(new Event("input", { bubbles: true }));
        nameInput.dispatchEvent(new Event("change", { bubbles: true }));
      }
      [...(section?.querySelectorAll("button") ?? [])].find((button) => button.textContent === "Save name")?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(invoke).toHaveBeenCalledWith("rename_credential", {
      id: "credential-1", name: "Production deploy", expectedRevision: 3,
    });

    await act(async () => {
      [...(section?.querySelectorAll("button") ?? [])].find((button) => button.textContent === "Update password")?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(invoke).toHaveBeenCalledWith("update_password_credential", {
      id: "credential-1", expectedRevision: 3, locale: "en",
    });

    await act(async () => {
      [...(section?.querySelectorAll("button") ?? [])].find((button) => button.textContent === "Delete")?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    await answerConfirmation("Delete Deploy");
    expect(invoke).toHaveBeenCalledWith("delete_credential", { id: "credential-1", expectedRevision: 3 });
  });

  it("reviews backup conflicts before merge and imports with the preview revision", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "vault_status") return { exists: true, unlocked: true, revision: 3, epoch: 1 };
      if (command === "list_hosts") return [{ ...host }];
      if (command === "list_credentials") return [{ id: "credential-1", name: "Deploy", revision: 0, kind: "password" }];
      if (command === "list_groups") return groups.map((item) => ({ ...item }));
      if (command === "list_trusted_host_keys") return [];
      if (command === "vault_preferences") return { auto_lock_minutes: 30, disconnect_on_lock: false };
      if (command === "preview_vault_backup") return {
        token: "short-lived-token",
        preview: {
          baseRevision: 3, hostCount: 2, groupCount: 1, credentialCount: 1, trustCount: 1,
          duplicateHosts: [{ existingName: "Build Server", incomingName: "Imported build", target: "build.example.test", port: 22 }],
          trustConflicts: [],
        },
      };
      return 4;
    });
    await openSettings();
    const previewButton = [...container.querySelectorAll("button")].find((button) => button.textContent?.includes("Preview and import backup"));
    expect(previewButton).not.toBeUndefined();
    await act(async () => {
      previewButton?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(container.textContent).toContain("Imported build → Build Server");
    expect(container.textContent).toContain("Backup contains 2 hosts");
    const merge = [...container.querySelectorAll("button")].find((button) => button.textContent?.includes("Merge new entries"));
    expect(merge?.disabled).toBe(false);
    await act(async () => {
      merge?.click();
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    expect(invoke).toHaveBeenCalledWith("import_vault_backup", {
      token: "short-lived-token", mode: "merge", expectedRevision: 3, expectedEpoch: 1,
    });
  });

  it("keeps vault management in one dismissible settings sheet", async () => {
    expect(container.querySelector(".sheet")).toBeNull();
    const trigger = container.querySelector<HTMLButtonElement>("button[aria-label='Settings']");
    trigger?.focus();
    await openSettings();
    const sheet = container.querySelector<HTMLElement>(".sheet");
    expect(sheet?.getAttribute("role")).toBe("dialog");
    expect(trigger?.getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement?.textContent).toBe("Security");
    await act(async () => {
      sheet?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await new Promise((resolve) => window.requestAnimationFrame(resolve));
    });
    expect(container.querySelector(".sheet")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("groups favorites first and explains when a search matches nothing", async () => {
    expect(container.querySelector(".host-group-label")?.textContent).toContain("Ungrouped");
    const search = container.querySelector<HTMLInputElement>("input[aria-label='Search hosts']");
    await act(async () => {
      if (search) {
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
        setter?.call(search, "no-such-host");
        search.dispatchEvent(new Event("input", { bubbles: true }));
      }
    });
    expect(container.querySelector(".host-row")).toBeNull();
    expect(container.textContent).toContain("No hosts match “no-such-host”.");
  });

  it("asks before deleting a host and keeps it when the confirmation is canceled", async () => {
    await act(async () => { container.querySelector<HTMLButtonElement>(".host-edit")?.click(); });
    await act(async () => { bodyButton("Delete host")?.click(); });
    expect(await answerConfirmation(null)).toContain("Build Server");
    expect(invoke).not.toHaveBeenCalledWith("delete_host", expect.anything());
    await act(async () => { bodyButton("Delete host")?.click(); });
    await answerConfirmation("Delete Build Server");
    expect(invoke).toHaveBeenCalledWith("delete_host", { id: "host-1", expectedRevision: 3 });
  });

  it("names missing host fields inline instead of submitting", async () => {
    await act(async () => { container.querySelector<HTMLButtonElement>("button[aria-label='Add host']")?.click(); });
    const editor = container.querySelector<HTMLFormElement>("form.host-editor");
    await act(async () => {
      editor?.querySelector<HTMLButtonElement>('button[type="submit"]')?.click();
      await new Promise((resolve) => window.requestAnimationFrame(resolve));
    });
    expect(invoke).not.toHaveBeenCalledWith("save_host", expect.anything());
    expect(editor?.querySelectorAll("[aria-invalid='true']")).toHaveLength(4);
    expect(editor?.textContent).toContain("Required");
    expect(document.activeElement?.getAttribute("aria-invalid")).toBe("true");
  });

  it("marks production hosts and focuses an open host instead of duplicating its tab", async () => {
    invoke.mockImplementation(async (command: string, args?: unknown) => command === "list_hosts"
      ? [{ ...host, tags: ["prod"] }, { ...host, id: "host-2", name: "Lab box", group_id: "group-1" }]
      : defaultInvoke(command, args));
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => {
      root.render(<App />);
      await new Promise((resolve) => window.setTimeout(resolve, 0));
    });
    const rows = [...container.querySelectorAll<HTMLElement>(".host-row")];
    expect(rows.map((row) => row.querySelector(".host-key")?.textContent)).toEqual(["LB", "BS"]);
    expect(container.querySelectorAll(".host-group-label")[1]?.textContent).toContain("Ungrouped");
    const production = rows.find((row) => row.textContent?.includes("Build Server"));
    expect(production?.dataset.prod).toBe("true");
    const open = production?.querySelector<HTMLButtonElement>(".host-open");
    await act(async () => { open?.click(); });
    await act(async () => { open?.click(); });
    const sshTabs = [...container.querySelectorAll<HTMLElement>(".terminal-tab")].filter((tab) => tab.textContent?.includes("Build Server"));
    expect(sshTabs).toHaveLength(1);
    expect(sshTabs[0].dataset.prod).toBe("true");
    expect(sshTabs[0].querySelector("[role='tab']")?.getAttribute("aria-label")).toContain("Production");
    await act(async () => { open?.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true })); });
    expect([...container.querySelectorAll(".terminal-tab")].filter((tab) => tab.textContent?.includes("Build Server"))).toHaveLength(2);
  });

  it("inherits production from a parent group and saves the host's own switch", async () => {
    invoke.mockImplementation(async (command: string, args?: unknown) => command === "list_hosts"
      ? [{ ...host, group_id: "group-1" }, { ...host, id: "host-2", name: "Lab box" }]
      : command === "list_groups" ? [{ ...group, parent_id: "group-3" }, { id: "group-3", name: "Live", parent_id: null, production: true }]
        : defaultInvoke(command, args));
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => { root.render(<App />); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    const rows = [...container.querySelectorAll<HTMLElement>(".host-row")];
    expect(rows.find((row) => row.textContent?.includes("Build Server"))?.dataset.prod).toBe("true");
    expect(rows.find((row) => row.textContent?.includes("Lab box"))?.dataset.prod).toBeUndefined();
    await act(async () => { container.querySelector<HTMLButtonElement>("button[aria-label='Edit host: Build Server']")?.click(); });
    expect(container.querySelector("#host-production-hint")?.textContent).toBe("Already production through the group Live.");
    await act(async () => { bodyButton("Cancel")?.click(); });
    await act(async () => { container.querySelector<HTMLButtonElement>("button[aria-label='Edit host: Lab box']")?.click(); });
    const production = [...container.querySelectorAll<HTMLLabelElement>("form.host-editor label")].find((label) => label.textContent === "Production host")?.querySelector("input");
    await act(async () => { production?.click(); });
    await act(async () => { container.querySelector<HTMLButtonElement>("form.host-editor button[type='submit']")?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    expect(invoke).toHaveBeenCalledWith("save_host", expect.objectContaining({ host: expect.objectContaining({ id: "host-2", production: true }) }));
  });

  it("reaches the host list from anywhere and walks it with the keyboard", async () => {
    invoke.mockImplementation(async (command: string, args?: unknown) => command === "list_hosts"
      ? [{ ...host }, { ...host, id: "host-2", name: "Lab box" }]
      : defaultInvoke(command, args));
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => { root.render(<App />); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "K", ctrlKey: true, shiftKey: true, bubbles: true }));
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });
    expect(document.activeElement?.getAttribute("aria-label")).toBe("Search hosts");
    const openers = [...container.querySelectorAll<HTMLButtonElement>(".host-open")];
    expect(openers.map((button) => button.tabIndex)).toEqual([0, -1]);
    await act(async () => { document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })); });
    expect(document.activeElement).toBe(openers[0]);
    await act(async () => { openers[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true })); });
    expect(document.activeElement).toBe(openers[1]);
    expect(container.querySelectorAll<HTMLButtonElement>(".host-open")[1]?.tabIndex).toBe(0);
    await act(async () => { openers[1].dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true })); });
    expect(container.querySelector<HTMLInputElement>("form.host-editor input")?.value).toBe("Lab box");
  });

  it("hides the host list on wide windows and remembers it", async () => {
    const toggle = container.querySelector<HTMLButtonElement>(".sidebar-toggle");
    expect(toggle?.getAttribute("aria-expanded")).toBe("true");
    await act(async () => { toggle?.click(); });
    expect(container.querySelector(".app-layout")?.classList.contains("sidebar-hidden")).toBe(true);
    expect(localStorage.getItem("capstan.sidebar-hidden")).toBe("1");
    await act(async () => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "B", ctrlKey: true, shiftKey: true, bubbles: true })); });
    expect(container.querySelector(".app-layout")?.classList.contains("sidebar-hidden")).toBe(false);
    expect(localStorage.getItem("capstan.sidebar-hidden")).toBe("0");
  });

  it("keeps errors visible while the host list is hidden", async () => {
    await act(async () => { container.querySelector<HTMLButtonElement>(".sidebar-toggle")?.click(); });
    invoke.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "lock_vault") throw new Error("lock failed");
      return defaultInvoke(command, args);
    });
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "p", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true }));
    });
    const lock = [...document.body.querySelectorAll<HTMLButtonElement>(".command-result")].find((button) => button.textContent?.includes("Lock vault"));
    expect(lock).toBeDefined();
    await act(async () => { lock?.click(); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    expect(container.querySelector(".sidebar .inline-error")).toBeNull();
    expect(container.querySelector(".workspace-messages .inline-error[role='alert']")).not.toBeNull();
    await act(async () => { container.querySelector<HTMLButtonElement>(".sidebar-toggle")?.click(); });
    expect(container.querySelector(".workspace-messages")).toBeNull();
    expect(container.querySelector(".sidebar .inline-error[role='alert']")).not.toBeNull();
  });

  it("places the production switch in the connection section", async () => {
    await act(async () => { container.querySelector<HTMLButtonElement>("button[aria-label='Edit host: Build Server']")?.click(); });
    const connection = container.querySelector("form.host-editor .form-section");
    expect(connection?.querySelector("h3")?.textContent).toBe("Connection");
    expect(connection?.querySelector(".production-row input[type='checkbox']")).not.toBeNull();
  });

  it("marks production hosts in the palette and runs workspace actions through the same paths", async () => {
    invoke.mockImplementation(async (command: string, args?: unknown) => command === "list_hosts"
      ? [{ ...host, tags: ["prod"], port: 2222 }]
      : defaultInvoke(command, args));
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => { root.render(<App />); await new Promise((resolve) => window.setTimeout(resolve, 0)); });
    const openPalette = async (): Promise<void> => act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "p", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true }));
    });
    const result = (text: string): HTMLButtonElement | undefined =>
      [...document.body.querySelectorAll<HTMLButtonElement>(".command-result")].find((button) => button.textContent?.includes(text));
    await openPalette();
    const hostAction = result("Build Server");
    expect(hostAction?.querySelector(".prod-chip")?.textContent).toBe("Production");
    expect(hostAction?.querySelector(".command-detail")?.textContent).toBe("deploy@build.example.test:2222");
    expect(hostAction?.querySelector(".command-label")?.textContent).toBe("Build Server");
    expect(result("Light terminal theme")?.textContent).toContain("All panes in every open tab");
    expect(result("Open SSH tunnels")?.textContent).not.toContain("snippets");
    const activeTab = () => useWorkspace.getState().tabs.find((tab) => tab.id === useWorkspace.getState().activeTab)!;
    expect(paneLeaves(activeTab().root)).toHaveLength(1);
    await act(async () => { result("Split current pane right")?.click(); });
    expect(paneLeaves(activeTab().root)).toHaveLength(2);
    await openPalette();
    const tabCount = useWorkspace.getState().tabs.length;
    const newTerminal = result("New local terminal");
    expect(newTerminal?.querySelector("kbd")?.textContent).toBe("Ctrl+Shift+T");
    await act(async () => { newTerminal?.click(); });
    expect(useWorkspace.getState().tabs).toHaveLength(tabCount + 1);
  });

  it("lists the keyboard shortcuts in settings", async () => {
    await openSettings("Shortcuts");
    const keys = [...container.querySelectorAll(".shortcut-list kbd")].map((key) => key.textContent);
    expect(keys).toContain("Ctrl+Shift+K");
    expect(keys).toContain("Ctrl+Shift+P");
    expect(keys).toContain("F6 / Shift+F6");
  });

  it("opens the registered command palette and connects only a saved host action", async () => {
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "p", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true }));
    });
    expect(document.body.querySelector(".command-palette")).not.toBeNull();
    const search = document.body.querySelector<HTMLInputElement>(".command-palette input[type='search']");
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(search, "Build Server");
      search?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const action = document.body.querySelector<HTMLButtonElement>(".command-result[role='option']");
    expect(action?.textContent).toContain("Build Server");
    expect(action?.querySelector(".prod-chip")).toBeNull();
    await act(async () => { action?.click(); });
    const current = useWorkspace.getState().tabs.find((tab) => tab.id === useWorkspace.getState().activeTab);
    expect(current?.launch).toMatchObject({ kind: "ssh", host_id: "host-1" });
  });

  it("offers to add a typed user@host when the search matches nothing", async () => {
    const search = container.querySelector<HTMLInputElement>("input[aria-label='Search hosts']");
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(search, "ops@db1.internal:2222");
      search?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => { bodyButton("Add host ops@db1.internal:2222")?.click(); });
    const fields = [...container.querySelectorAll<HTMLInputElement>("form.host-editor input")].map((input) => input.value);
    expect(fields.slice(0, 4)).toEqual(["db1.internal", "db1.internal", "2222", "ops"]);
  });

  it("moves between terminal tabs with Windows Terminal shortcuts and arrow keys", async () => {
    await act(async () => { const state = useWorkspace.getState(); for (const tab of state.tabs) state.closeTerminal(tab.id); });
    const before = container.querySelectorAll(".terminal-tab").length;
    expect(before).toBe(1);
    await act(async () => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "T", ctrlKey: true, shiftKey: true, bubbles: true })); });
    const tabs = [...container.querySelectorAll<HTMLButtonElement>(".terminal-tab [role='tab']")];
    expect(tabs).toHaveLength(before + 1);
    expect(tabs.at(-1)?.getAttribute("aria-selected")).toBe("true");
    expect(tabs.map((tab) => tab.tabIndex)).toEqual([...Array(before).fill(-1), 0]);
    expect(tabs.map((tab) => tab.textContent)).toEqual(["PowerShell 1", "PowerShell 2"]);
    await act(async () => { tabs.at(-1)?.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true })); });
    expect(container.querySelector(".terminal-tab [role='tab']")?.getAttribute("aria-selected")).toBe("true");
    await act(async () => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "W", ctrlKey: true, shiftKey: true, bubbles: true })); });
    expect(container.querySelectorAll(".terminal-tab")).toHaveLength(before);
  });

  it("exposes nested split actions and reorders tabs by local drag and keyboard without changing pane state", async () => {
    await act(async () => { const state = useWorkspace.getState(); for (const tab of state.tabs) state.closeTerminal(tab.id); });
    const first = useWorkspace.getState().tabs[0];
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Split pane right']")!.click());
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Split pane down']")!.click());
    const split = useWorkspace.getState().tabs[0];
    expect(paneLeaves(split.root)).toHaveLength(3);
    expect(split.root.kind === "split" && split.root.second.kind).toBe("split");
    await act(async () => useWorkspace.getState().addTerminal());
    const second = useWorkspace.getState().tabs[1];
    const entries: Record<string, string> = {};
    const elements = container.querySelectorAll(".terminal-tab");
    await act(async () => {
      elements[0].dispatchEvent(dragEvent("dragstart", entries));
      elements[1].dispatchEvent(dragEvent("drop", entries));
    });
    expect(useWorkspace.getState().tabs.map((tab) => tab.id)).toEqual([second.id, first.id]);
    expect(useWorkspace.getState().tabs[1]).toBe(split);
    const tabButton = container.querySelector<HTMLButtonElement>(`button[id='${first.id}-tab']`)!;
    await act(async () => tabButton.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", altKey: true, shiftKey: true, bubbles: true })));
    expect(useWorkspace.getState().tabs.map((tab) => tab.id)).toEqual([first.id, second.id]);
    expect(useWorkspace.getState().tabs[0]).toBe(split);
    // A forged external drop without a local drag cannot reorder the workspace.
    await act(async () => elements[0].dispatchEvent(dragEvent("drop", { "application/x-capstan-tab": second.id })));
    expect(useWorkspace.getState().tabs[0]).toBe(split);
  });

  it("follows the focused SSH pane for SFTP and keeps another shell when a pane closes", async () => {
    await act(async () => useWorkspace.getState().addSshTerminal(host.id as HostId, host.name, "graphite"));
    const tab = useWorkspace.getState().tabs.at(-1)!, first = tab.focusedPaneId;
    const metrics: TerminalSnapshot["metrics"] = { queued_bytes: 0, in_flight_bytes: 0, max_outstanding_bytes: 0, input_bytes: 0, output_bytes: 0,
      frame_count: 0, max_pending_bytes: 0, max_in_flight_bytes: 0, pause_count: 0, resume_count: 0, canceled: false, channel_closed: false };
    const snapshot = (id: string): TerminalSnapshot => ({ id: id as TerminalSessionId, state: "running", exit_code: null, error_code: null, metrics });
    await act(async () => {
      useWorkspace.getState().updateSession(tab.id, snapshot("ssh-first"), first);
      useWorkspace.getState().splitPane(tab.id, first, "horizontal");
    });
    const current = useWorkspace.getState().tabs.find((item) => item.id === tab.id)!, second = current.focusedPaneId;
    await act(async () => useWorkspace.getState().updateSession(tab.id, snapshot("ssh-second"), second));
    expect(invoke).toHaveBeenCalledWith("attach_sftp", { terminal: "ssh-second" });
    await act(async () => useWorkspace.getState().focusPane(tab.id, first));
    expect(invoke).toHaveBeenCalledWith("attach_sftp", { terminal: "ssh-first" });
    await act(async () => useWorkspace.getState().closePane(tab.id, first));
    const after = useWorkspace.getState().tabs.find((item) => item.id === tab.id)!;
    expect(after.paneStates[second]?.session?.id).toBe("ssh-second");
    expect(paneLeaves(after.root)).toHaveLength(1);
    expect(invoke).not.toHaveBeenCalledWith("stop_terminal", { id: "ssh-second" });
  });

  it("keeps pane effects mounted during tab reorder in React StrictMode", async () => {
    await act(async () => root.unmount());
    const state = useWorkspace.getState();
    for (const tab of state.tabs) state.closeTerminal(tab.id);
    root = createRoot(container);
    await act(async () => root.render(<StrictMode><App /></StrictMode>));
    const first = useWorkspace.getState().tabs[0];
    await act(async () => useWorkspace.getState().splitPane(first.id, first.focusedPaneId, "horizontal"));
    await act(async () => useWorkspace.getState().addTerminal());
    const second = useWorkspace.getState().tabs[1];
    const started = lifecycle.started.length, stopped = lifecycle.stopped.length;
    const panels = [...container.querySelectorAll(".terminal-pane")];
    await act(async () => useWorkspace.getState().moveTab(first.id, second.id));
    expect(useWorkspace.getState().tabs.map((tab) => tab.id)).toEqual([second.id, first.id]);
    expect([...container.querySelectorAll(".terminal-pane")]).toEqual(panels);
    expect(lifecycle.started).toHaveLength(started);
    expect(lifecycle.stopped).toHaveLength(stopped);
  });
});

const runSnippet = {
  id: "11111111-1111-4111-8111-111111111111", group_id: null,
  name: "Review command", template: "echo review", auto_enter: false,
} as Snippet;

async function mountSnippetApp(strict = false, autoEnter = false): Promise<void> {
  invoke.mockImplementation(async (command, args) => command === "list_snippets"
    ? [{ ...runSnippet, auto_enter: autoEnter }] : defaultInvoke(command, args));
  await act(async () => root.unmount());
  useWorkspace.setState(useWorkspace.getInitialState());
  root = createRoot(container);
  await act(async () => {
    root.render(strict ? <StrictMode><App /></StrictMode> : <App />);
    await new Promise((resolve) => window.setTimeout(resolve, 0));
  });
}

function activeSnippetTab() {
  const state = useWorkspace.getState();
  return state.tabs.find((tab) => tab.id === state.activeTab)!;
}

async function openSnippetPalette(): Promise<void> {
  await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "p", ctrlKey: true, shiftKey: true, bubbles: true })));
  expect(document.body.querySelector(".command-palette")).not.toBeNull();
}

async function chooseSnippetAction(): Promise<void> {
  const action = [...document.body.querySelectorAll<HTMLButtonElement>(".command-result")]
    .find((button) => button.textContent?.includes(runSnippet.name))!;
  expect(action).toBeDefined();
  await act(async () => { action.click(); });
}

async function openSnippetBroadcast(): Promise<void> {
  await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Open command snippets']")!.click());
  const select = container.querySelector<HTMLSelectElement>(".snippet-target select")!;
  await act(async () => { select.value = "broadcast"; select.dispatchEvent(new Event("change", { bubbles: true })); });
}

describe("App snippet delivery boundaries", () => {
  it("refreshes after a vault conflict and preserves the snippet draft for an explicit retry", async () => {
    await mountSnippetApp();
    let revision = 3, name = runSnippet.name;
    const saved: { snippet: Snippet; expectedRevision: string }[] = [];
    invoke.mockImplementation(async (command, args) => {
      if (command === "vault_status") return { exists: true, unlocked: true, revision, epoch: 1 };
      if (command === "list_snippets") return [{ ...runSnippet, name }];
      if (command === "save_snippet") {
        const request = args as { snippet: Snippet; expectedRevision: string };
        saved.push(request);
        if (saved.length === 1) { revision = 4; name = "Concurrent update"; throw { code: "vault_conflict" }; }
        revision = 5; name = request.snippet.name;
        return "5";
      }
      return defaultInvoke(command, args);
    });
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Open command snippets']")!.click());
    await act(async () => container.querySelector<HTMLButtonElement>(`button[aria-label='Edit snippet: ${runSnippet.name}']`)!.click());
    const editor = document.body.querySelector<HTMLFormElement>(".snippet-editor")!;
    const input = editor.querySelector<HTMLInputElement>("input")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "My preserved draft");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => editor.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(saved).toHaveLength(1);
    expect(saved[0].expectedRevision).toBe("3");
    expect(editor.querySelector("[role='alert']")?.textContent).toBe(copy.en.snippetConflict);
    expect(input.value).toBe("My preserved draft");
    expect(container.querySelector(".snippet-run")?.textContent).toContain("Concurrent update");
    expect(container.querySelector(".message-stack")?.textContent).not.toContain(copy.en.connectionFailed);
    await act(async () => editor.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(saved).toHaveLength(2);
    expect(saved[1]).toMatchObject({ expectedRevision: "4", snippet: { name: "My preserved draft" } });
    expect(document.body.querySelector(".snippet-editor")).toBeNull();
  });

  it("closes only the snippet editor on Escape and keeps the sheet open", async () => {
    await mountSnippetApp();
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Open command snippets']")!.click());
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='New snippet']")!.click());
    const input = document.body.querySelector<HTMLInputElement>(".snippet-editor input")!;
    expect(document.body.querySelector(".snippet-editor")?.getAttribute("role")).toBe("dialog");
    await act(async () => { input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); });
    expect(document.body.querySelector(".snippet-editor")).toBeNull();
    expect(container.querySelector(".snippet-manager")).not.toBeNull();
  });

  it("asks before deleting a snippet and deletes nothing on Cancel", async () => {
    await mountSnippetApp();
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Open command snippets']")!.click());
    const remove = () => container.querySelector<HTMLButtonElement>(`button[aria-label='Delete snippet: ${runSnippet.name}']`)!;
    await act(async () => remove().click());
    expect(await answerConfirmation(null)).toContain(copy.en.confirmDeleteSnippetTitle(runSnippet.name));
    expect(invoke).not.toHaveBeenCalledWith("delete_snippet", expect.anything());
    await act(async () => remove().click());
    await answerConfirmation(copy.en.deleteNamed(runSnippet.name));
    expect(invoke).toHaveBeenCalledWith("delete_snippet", expect.objectContaining({ id: runSnippet.id }));
  });

  it("opens the palette over the snippets sheet but not over the host editor", async () => {
    await mountSnippetApp();
    const palette = async (): Promise<void> => act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "P", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true }));
    });
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Open command snippets']")!.click());
    await palette();
    expect(document.body.querySelector(".command-palette")).not.toBeNull();
    await act(async () => document.body.querySelector<HTMLInputElement>(".command-palette input")!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(document.body.querySelector(".command-palette")).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-label='Add host']")!.click());
    expect(container.querySelector("form.host-editor")).not.toBeNull();
    await palette();
    expect(document.body.querySelector(".command-palette")).toBeNull();
  });

  it.each([true, false])("keeps session owners throughout a snippet broadcast confirmation (initially connected=%s)", async (initiallyConnected) => {
    await mountSnippetApp(true, true);
    const initial = activeSnippetTab();
    await act(async () => useWorkspace.getState().splitPane(initial.id, initial.focusedPaneId, "horizontal"));
    const tab = activeSnippetTab(), leaves = paneLeaves(tab.root);
    await act(async () => leaves.forEach((pane) => useWorkspace.getState().toggleBroadcastPane(tab.id, pane.paneId)));
    const good = vi.fn(async () => undefined), oldSession = vi.fn(async () => undefined), replacement = vi.fn(async () => undefined);
    const removeGood = paneInputs.register(leaves[0].paneId, good), removeOld = initiallyConnected ? paneInputs.register(leaves[1].paneId, oldSession) : () => undefined;
    let removeNew = () => undefined as void;
    try {
      await openSnippetBroadcast();
      await act(async () => container.querySelector<HTMLButtonElement>(".snippet-run")!.click());
      expect(good).not.toHaveBeenCalled();
      // Auto-reconnect keeps PaneId and TerminalLaunch, but replaces its input owner.
      removeNew = paneInputs.register(leaves[1].paneId, replacement);
      await answerConfirmation(copy.en.confirmBroadcastSend(2));
      expect(replacement).not.toHaveBeenCalled();
      expect(oldSession).not.toHaveBeenCalled();
      expect(good).toHaveBeenCalledTimes(1);
      expect(container.querySelector(".snippet-manager [role='alert']")?.textContent).toContain(`Pane 2 (${copy.en.paneInputClosed})`);
    } finally { removeGood(); removeOld(); removeNew(); }
  });

  it.each([false, true])("consumes each palette run once and allows a new request for the same snippet (StrictMode=%s)", async (strict) => {
    await mountSnippetApp(strict, true);
    const write = vi.fn(async (_bytes: Uint8Array) => undefined);
    const unregister = paneInputs.register(activeSnippetTab().focusedPaneId, write);
    try {
      expect(container.querySelector(".snippet-manager")).toBeNull();
      await openSnippetPalette();
      await chooseSnippetAction();
      expect(write).toHaveBeenCalledTimes(1);
      expect(new TextDecoder().decode(write.mock.calls[0][0])).toBe("echo review\r");
      expect(container.querySelector(".snippet-manager")).not.toBeNull();

      await act(async () => container.querySelector<HTMLButtonElement>(".snippet-run")!.click());
      expect(write).toHaveBeenCalledTimes(2);
      await act(async () => container.querySelector<HTMLButtonElement>(".snippet-manager button[aria-label='Close panel']")!.click());
      await openSnippetPalette();
      await chooseSnippetAction();
      expect(write).toHaveBeenCalledTimes(3);
      expect(write.mock.calls.map(([bytes]) => new TextDecoder().decode(bytes))).toEqual(Array(3).fill("echo review\r"));
    } finally { unregister(); }
  });

  it.each([
    ["session_closed", copy.en.paneInputClosed],
    ["input_queue_full", copy.en.paneInputQueueFull],
    ["backend", copy.en.paneInputUnknown],
  ])("shows a partial broadcast's failed pane and %s reason inside the open snippet panel", async (code, reason) => {
    await mountSnippetApp();
    const initial = activeSnippetTab();
    await act(async () => useWorkspace.getState().splitPane(initial.id, initial.focusedPaneId, "horizontal"));
    const tab = activeSnippetTab(), leaves = paneLeaves(tab.root);
    await act(async () => leaves.forEach((pane) => useWorkspace.getState().toggleBroadcastPane(tab.id, pane.paneId)));
    const good = vi.fn(async () => undefined), bad = vi.fn(async () => { throw { code }; });
    const removeGood = paneInputs.register(leaves[0].paneId, good), removeBad = paneInputs.register(leaves[1].paneId, bad);
    try {
      await openSnippetBroadcast();
      await act(async () => container.querySelector<HTMLButtonElement>(".snippet-run")!.click());
      expect(good).not.toHaveBeenCalled();
      expect(bad).not.toHaveBeenCalled();
      await answerConfirmation(copy.en.confirmBroadcastSend(2));
      expect(good).toHaveBeenCalledTimes(1);
      expect(bad).toHaveBeenCalledTimes(1);
      const alert = container.querySelector(".snippet-manager [role='alert']");
      expect(alert?.textContent).toContain(copy.en.snippetSendFailed);
      expect(alert?.textContent).toContain(`Pane 2 (${reason})`);
      expect(alert?.textContent).not.toContain("Pane 1");
      await act(async () => container.querySelector<HTMLInputElement>(".snippet-manager input[type='search']")!.focus());
      expect(good).toHaveBeenCalledTimes(1);
      expect(bad).toHaveBeenCalledTimes(1);
    } finally { removeGood(); removeBad(); }
  });

  it("shows total failure and an empty broadcast selection without closing the panel or replaying input", async () => {
    await mountSnippetApp();
    const tab = activeSnippetTab();
    await act(async () => useWorkspace.getState().toggleBroadcastPane(tab.id, tab.focusedPaneId));
    const write = vi.fn(async () => { throw { code: "session_closed" }; });
    const unregister = paneInputs.register(tab.focusedPaneId, write);
    try {
      await openSnippetBroadcast();
      await act(async () => container.querySelector<HTMLButtonElement>(".snippet-run")!.click());
      expect(write).not.toHaveBeenCalled();
      await answerConfirmation(copy.en.confirmBroadcastSend(1));
      expect(write).toHaveBeenCalledTimes(1);
      expect(container.querySelector(".snippet-manager [role='alert']")?.textContent).toContain(copy.en.snippetSendFailed);

      await act(async () => useWorkspace.getState().toggleBroadcastPane(tab.id, tab.focusedPaneId));
      await act(async () => container.querySelector<HTMLButtonElement>(".snippet-run")!.click());
      expect(write).toHaveBeenCalledTimes(1);
      expect(container.querySelector(".snippet-manager [role='alert']")?.textContent).toContain(copy.en.snippetNoBroadcastTargets);
      expect(container.querySelector<HTMLSelectElement>(".snippet-target select")?.disabled).toBe(false);
    } finally { unregister(); }
  });

  it.each([
    { isComposing: true },
    { keyCode: 229 },
  ])("keeps IME confirmation and navigation from executing a palette action: %j", async (composition) => {
    await mountSnippetApp(false, true);
    const write = vi.fn(async (_bytes: Uint8Array) => undefined);
    const unregister = paneInputs.register(activeSnippetTab().focusedPaneId, write);
    try {
      await openSnippetPalette();
      const initial = document.body.querySelector(".command-result[aria-selected='true']")?.textContent;
      const search = document.body.querySelector<HTMLInputElement>(".command-palette input")!;
      await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", ...composition, bubbles: true, cancelable: true })));
      expect(document.body.querySelector(".command-result[aria-selected='true']")?.textContent).toBe(initial);
      await act(async () => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, runSnippet.name);
        search.dispatchEvent(new Event("input", { bubbles: true }));
      });
      expect(document.body.querySelectorAll(".command-result")).toHaveLength(1);
      await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ...composition, bubbles: true, cancelable: true })));
      expect(write).not.toHaveBeenCalled();
      expect(document.body.querySelector(".command-palette")).not.toBeNull();
      expect(container.querySelector(".snippet-manager")).toBeNull();
      await act(async () => search.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
      expect(write).toHaveBeenCalledTimes(1);
      expect(new TextDecoder().decode(write.mock.calls[0][0])).toBe("echo review\r");
    } finally { unregister(); }
  });

  it.each([false, true])("confirms every snippet broadcast even for armed targets (autoEnter=%s)", async (autoEnter) => {
    await mountSnippetApp(true, autoEnter);
    const tab = activeSnippetTab();
    await act(async () => {
      useWorkspace.getState().setBroadcast(tab.id, true);
      useWorkspace.getState().toggleBroadcastPane(tab.id, tab.focusedPaneId);
      useWorkspace.getState().armBroadcast(tab.id, [tab.focusedPaneId]);
    });
    const write = vi.fn(async (_bytes: Uint8Array) => undefined);
    const unregister = paneInputs.register(tab.focusedPaneId, write);
    try {
      await openSnippetBroadcast();
      await act(async () => container.querySelector<HTMLButtonElement>(".snippet-run")!.click());
      expect(write).not.toHaveBeenCalled();
      await answerConfirmation(null);
      expect(write).not.toHaveBeenCalled();
      await act(async () => container.querySelector<HTMLButtonElement>(".snippet-run")!.click());
      expect(write).not.toHaveBeenCalled();
      await answerConfirmation(copy.en.confirmBroadcastSend(1));
      expect(write).toHaveBeenCalledTimes(1);
      expect(new TextDecoder().decode(write.mock.calls[0][0])).toBe(autoEnter ? "echo review\r" : "echo review");
      expect(container.querySelector(".snippet-manager [role='status']")?.textContent).toContain(copy.en.snippetSent);
      await act(async () => container.querySelector<HTMLButtonElement>(".snippet-run")!.click());
      expect(write).toHaveBeenCalledTimes(1);
      await answerConfirmation(null);
      expect(write).toHaveBeenCalledTimes(1);
    } finally { unregister(); }
  });
});
