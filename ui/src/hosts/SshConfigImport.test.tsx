import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { copy } from "../i18n";
import type { SshConfigHost, SshConfigPreview } from "../terminal/contracts";
import { SshConfigImportDialog } from "./SshConfigImport";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("../terminal/bridge", () => ({ invokeTauri: invoke }));
Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", { value: true, configurable: true });

const host = (index: number, overrides: Partial<SshConfigHost> = {}): SshConfigHost => ({
  index, alias: `host-${index}`, address: `10.0.0.${index}`, port: 22, username: "deploy",
  identityFile: null, encrypted: false, notes: [], saved: false, jump: null, ...overrides,
});
const preview: SshConfigPreview = {
  token: "token-1",
  hosts: [
    host(0, { identityFile: "/home/me/.ssh/id_ed25519" }),
    host(1, { identityFile: "C:\\Users\\me\\.ssh\\work_key", encrypted: true, port: 2222 }),
    host(2, { saved: true }),
    host(3, { notes: ["proxy_unsupported"] }),
  ],
  skipped: ["match_block"],
};

let root: Root;
let container: HTMLDivElement;
const onClose = vi.fn(), onImported = vi.fn();
const render = async (open = true) => act(async () => root.render(
  <SshConfigImportDialog open={open} labels={copy.en} locale="en" onClose={onClose} onImported={onImported} />,
));
const rows = () => [...document.body.querySelectorAll<HTMLLabelElement>(".ssh-import-row")];
const importButton = () => [...document.body.querySelectorAll<HTMLButtonElement>(".sftp-dialog-actions button")].find((button) => button.className === "primary-action");

beforeEach(() => {
  vi.clearAllMocks();
  invoke.mockReset();
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

describe("SSH config import", () => {
  it("preselects new direct hosts and sends only the token, indices and names", async () => {
    invoke.mockResolvedValueOnce(preview);
    await render();
    expect(invoke).toHaveBeenCalledExactlyOnceWith("preview_ssh_config_import");
    const checked = rows().map((row) => row.querySelector("input")!.checked);
    expect(checked).toEqual([true, true, false, false]);
    expect(rows()[2].querySelector("input")!.disabled).toBe(true);
    expect(rows()[0].textContent).toContain(copy.en.sshConfigKey("id_ed25519"));
    expect(rows()[1].textContent).toContain(copy.en.sshConfigKeyLocked("work_key"));
    expect(rows()[1].textContent).toContain("deploy@10.0.0.1:2222");
    expect(rows()[3].textContent).toContain(copy.en.sshConfigNotes.proxy_unsupported);
    expect(document.body.textContent).toContain(copy.en.sshConfigSkipped(1));

    await act(async () => rows()[3].querySelector("input")!.click());
    await act(async () => rows()[0].querySelector("input")!.click());
    expect(importButton()!.textContent).toBe(copy.en.sshConfigImport(2));
    const summary = { created: 2, existing: 0 };
    invoke.mockResolvedValueOnce(summary);
    await act(async () => importButton()!.click());
    expect(invoke).toHaveBeenLastCalledWith("import_ssh_config", {
      token: "token-1", indices: [1, 3], locale: "en",
      credentialName: copy.en.interactiveCredentialName, groupName: copy.en.sshConfigGroup,
    });
    expect(onImported).toHaveBeenCalledExactlyOnceWith(summary);
  });

  it("keeps a host and its jump hosts checked together", async () => {
    invoke.mockResolvedValueOnce({
      token: "token-2",
      hosts: [host(0, { alias: "db", jump: 1 }), host(1, { alias: "bastion", jump: 2 }), host(2, { alias: "edge" }), host(3, { alias: "saved-gw", saved: true }), host(4, { jump: 3 })],
      skipped: [],
    } satisfies SshConfigPreview);
    await render();
    expect(rows()[0].textContent).toContain(copy.en.sshConfigVia("bastion"));
    const checked = () => rows().map((row) => row.querySelector("input")!.checked);
    expect(checked()).toEqual([true, true, true, false, true]);
    // Unchecking a jump host unchecks the hosts reached through it.
    await act(async () => rows()[2].querySelector("input")!.click());
    expect(checked()).toEqual([false, false, false, false, true]);
    // Checking a host checks its unsaved jump hosts again.
    await act(async () => rows()[0].querySelector("input")!.click());
    expect(checked()).toEqual([true, true, true, false, true]);
    await act(async () => rows()[4].querySelector("input")!.click());
    await act(async () => rows()[4].querySelector("input")!.click());
    expect(checked()).toEqual([true, true, true, false, true]);
  });

  it("explains a missing config and an expired preview", async () => {
    invoke.mockResolvedValueOnce({ token: null, hosts: [], skipped: [] });
    await render();
    expect(document.body.textContent).toContain(copy.en.sshConfigMissing);
    expect(importButton()).toBeUndefined();

    await render(false);
    invoke.mockResolvedValueOnce(preview);
    await render();
    invoke.mockRejectedValueOnce({ code: "ssh_config_expired" });
    await act(async () => importButton()!.click());
    expect(document.body.querySelector("[role='alert']")?.textContent).toBe(copy.en.sshConfigExpired);
    expect(onImported).not.toHaveBeenCalled();
    // A declined passphrase dialog is not an error.
    invoke.mockRejectedValueOnce({ code: "canceled" });
    await act(async () => importButton()!.click());
    expect(document.body.querySelector("[role='alert']")).toBeNull();
    expect(importButton()!.disabled).toBe(false);
  });
});
