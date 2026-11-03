// Isolated UI fixture, built into target only. It has no native/file capability.
import { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { SftpPanel } from "../src/sftp/SftpPanel";
import { TreeDeleteDialog } from "../src/sftp/TreeDeleteDialog";
import deleteFixture from "../../docs/agent/fixtures/M2-delete.json";
import type { RemoteEntryId, SftpSessionId } from "../src/sftp/contracts";
import { TransferQueue } from "../src/transfers/TransferQueue";
import { TreeTransferDialog } from "../src/transfers/TreeTransferDialog";
import { decodeTreePlan } from "../src/transfers/tree";
import treeFixture from "../../docs/agent/fixtures/M2-tree.json";
import { RemoteEditors } from "../src/editors/RemoteEditors";
import editorFixture from "../../docs/agent/fixtures/M2-editor.json";
import { createTerminalSession } from "../src/terminal/session";
import { cwdHints } from "../src/sftp/cwdHints";
import type { TerminalSessionId } from "../src/terminal/contracts";
import "../src/styles.css";
import "@xterm/xterm/css/xterm.css";
const query = new URLSearchParams(location.search);
const locale = query.get("locale") === "zh-CN" ? "zh-CN" : "en";
const theme = query.get("theme") === "light" ? "light" : "graphite";
const session = "00000000-0000-4000-8000-000000000001";
const host = "00000000-0000-4000-8000-000000000002";
const terminal = "fixture-terminal" as TerminalSessionId;
const cwd = { sequence: "0", path: null as string | null, aliases: [] as string[] };
let cwdCallbacks = 0, acknowledgedBytes = 0, writtenBytes = 0;
let emitPreferenceExit: (() => void) | undefined;
class FixtureChannel<T> { onmessage: ((message: T) => void) | null = null; }
const tasks = [{ id: "00000000-0000-4000-8000-000000000003", hostId: host, name: "中文 archive.tar.gz", remotePath: "/home/test/中文 archive.tar.gz",
  direction: "upload", state: "awaiting_conflict", version: "1", processed: "1048576", total: "1048576", bytesPerSecond: "8388608", remainingSeconds: "0", error: null,
  conflict: { revision: "1", target: { kind: "file", size: "524288", modifiedSeconds: "1700000000", permissions: 33152 }, replaceAllowed: query.has("batch"), batchEligible: query.has("batch") }, cleanupRequired: false,
  batch: query.has("batch") ? { id: "00000000-0000-4000-8000-000000000004", name: "中文 config folder", files: 5, conflicts: 4 } : null }];
window.__TAURI__ = {
  event: { listen: async <T,>(name: string, callback: (event: { payload: T }) => void) => {
    if (name === "transfer-exit-requested") emitPreferenceExit = () => callback({ payload: { error: "settings_unavailable" } as T });
    return () => { if (name === "transfer-exit-requested") emitPreferenceExit = undefined; };
  } },
  core: { Channel: FixtureChannel, invoke: async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
    let result: unknown;
    if (command === "attach_sftp") result = { sessionId: session, hostId: host, connectionGeneration: "7", path: "/home/test", atomicReplace: false, fsync: true, requestBytes: 32768 };
    else if (command === "list_remote_directory") result = { sessionId: session, requestRevision: "1", pageSequence: 0, path: args?.path ?? "/home/test", hasMore: false, omittedEntries: 1,
      entries: ["config", "中文 archive.tar.gz", "notes.txt"].map((name, index) => ({ id: `00000000-0000-4000-8000-00000000001${index}`, name,
        metadata: { kind: index === 0 ? "directory" : "file", size: index === 0 ? null : "1048576", modifiedSeconds: "1700000000", permissions: index === 0 ? 16877 : 33152 } })) };
    else if (command === "transfer_snapshot") result = tasks;
    else if (command === "prepare_remote_tree_delete") result = { ...deleteFixture.summary, path: `/home/test/${"中文-long-directory-".repeat(30)}` };
    else if (command === "apply_remote_tree_delete") result = deleteFixture.report;
    else if (command === "native_export_snapshot") result = query.has("native-export") ? [{ token: treeFixture.token, name: "中文 export folder", state: "retained" }] : [];
    else if (command === "transfer_recovery_snapshot") result = { entries: query.has("manual-recovery") || query.has("cleanup-recovery") || query.has("upload-recovery") ? [{
      id: "00000000-0000-4000-8000-000000000020", hostId: host, name: query.has("upload-recovery") ? "old 中文 upload.bin" : "old 中文 download.bin", remotePath: query.has("upload-recovery") ? "/home/test/old 中文 upload.bin" : "/home/test/old 中文 download.bin",
      direction: query.has("upload-recovery") ? "upload" : "download", total: "1073741824", partialSize: "536870912", localPartial: !query.has("upload-recovery"), state: query.has("upload-recovery") ? "awaiting_reconnect" : query.has("cleanup-recovery") ? "cleanup_required" : "manual_inspection", updatedSeconds: "1",
    }] : [], incomplete: false };
    else if (command === "remote_edit_snapshot") result = query.has("editor-corrupt")
      ? { ...editorFixture, sessions: [{ ...editorFixture.sessions[1], error: "editor_residual" }] } : editorFixture;
    else if (command === "update_remote_cwd") {
      if (args?.terminal !== terminal || args.session !== session || args.generation !== "7") throw { code: "scope_expired" };
      const uri = typeof args.uri === "string" ? args.uri : "";
      if (uri.startsWith("file://build-server/")) { cwd.sequence = String(args.sequence); cwd.path = decodeURIComponent(uri.slice("file://build-server".length)); }
      result = { ...cwd };
    }
    else if (command === "configure_remote_cwd_aliases") {
      if (query.has("alias-error") && Array.isArray(args?.aliases) && args.aliases.includes("user@host")) throw { code: "path_invalid" };
      if (Array.isArray(args?.aliases)) cwd.aliases = args.aliases.filter((value): value is string => typeof value === "string");
      result = { ...cwd };
    }
    else if (command === "select_transfer_source" || command === "select_transfer_destination") result = null;
    return result as T;
  } },
};
function Fixture() {
  const terminalRef = useRef<HTMLDivElement>(null);
  const [layout, setLayout] = useState({ dock: query.get("dock") === "bottom" ? "bottom" as const : "right" as const, size: 40, collapsed: query.has("collapsed") });
  const [preferenceError, setPreferenceError] = useState<"load" | "save" | null>(query.has("preferences-load") ? "load" : query.has("preferences-save") ? "save" : null);
  useEffect(() => {
    const element = terminalRef.current;
    if (!element) return;
    const publisher = cwdHints.open(terminal);
    const view = createTerminalSession(element, true, {
      theme, onRemoteCwd: (uri) => { cwdCallbacks++; publisher.publish(uri); },
      ackOutput: (bytes) => { acknowledgedBytes += bytes; },
    });
    view.setVisible(true);
    view.writeLocal(new TextEncoder().encode("UI fixture · no SSH or filesystem access\r\n\r\nuser@build-server:~$ ls\r\nconfig  notes.txt  中文 archive.tar.gz\r\n\r\nuser@build-server:~$ "));
    if (query.has("cwd")) {
      // Real xterm parsing of binary input, with a split OSC 7 frame. The
      // fixture's IPC is metadata-only; authoritative validation is in Rust tests.
      const bytes = new TextEncoder().encode("\u001b]7;file://build-server/home/test/%E4%B8%AD%20folder\u001b\\");
      writtenBytes = bytes.length;
      view.write(bytes.slice(0, 17)); view.write(bytes.slice(17));
    }
    return () => { publisher.close(); view.dispose(); };
  }, []);
  return <div className="workspace" style={{ gridTemplateRows: query.has("editor") ? "34px minmax(0, 1fr) auto auto" : "34px minmax(0, 1fr) auto", background: "#16171a" }}>
    <div className="terminal-tabs"><span>Build server</span></div>
    <section className="terminal-shell"><div className={`terminal-pane terminal-pane-sftp-${layout.dock}${layout.collapsed ? " is-collapsed" : ""}`} style={{ "--sftp-size": `${layout.size}%` } as React.CSSProperties}>
      <div className="terminal-pane-terminal" ref={terminalRef} />
      <SftpPanel terminal={terminal} locale={locale} hostLabel="Build server" theme={theme} layout={layout} onLayoutChange={setLayout} preferencesError={preferenceError} onRetryPreferences={() => setPreferenceError(null)} />
    </div></section>
    {query.has("editor") && <RemoteEditors locale={locale} theme={theme} hosts={{ [editorFixture.sessions[0].hostId!]: "Build server" }} />}
    <TransferQueue locale={locale} theme={theme} hosts={{ [host]: "Build server" }} />
    {query.has("delete-tree") && <TreeDeleteDialog terminal={terminal} session={session as SftpSessionId} entry={"00000000-0000-4000-8000-000000000010" as RemoteEntryId} hostLabel="Build server" locale={locale} theme={theme} onClose={() => undefined} onChanged={() => undefined} />}
    {query.has("tree-plan") && <TreeTransferDialog plan={decodeTreePlan({ ...treeFixture, conflicts: treeFixture.conflicts.map((item) => ({ ...item, directory: true, source: `${item.source}-${"x".repeat(230)}`, kind: "directory_exists" })) })!} hostLabel="Build server" locale={locale} theme={theme} pending={false} error={null} onDecision={() => undefined} onClose={() => undefined} />}
  </div>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
const settle = () => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
async function check() {
  await document.fonts.ready;
  for (let attempt = 0; attempt < 60 && !document.querySelector(".transfer-row"); attempt++) await new Promise((resolve) => setTimeout(resolve, 50));
  await settle();
  const errors: string[] = [];
  if ((query.has("preferences-save") || query.has("preferences-load")) && !query.has("collapsed")) {
    if (!document.querySelector(".sftp-controls [role='alert']") || ![...document.querySelectorAll(".sftp-controls button")].some((button) => button.textContent === (locale === "zh-CN" ? "重试" : "Retry preferences"))) errors.push("Preference recovery missing");
  }
  if (query.has("preferences-save") && query.has("collapsed") && !document.querySelector(".sftp-panel-collapsed svg")) errors.push("Collapsed persistence warning missing");
  if (query.has("cwd")) {
    for (let attempt = 0; attempt < 60 && document.querySelector<HTMLInputElement>('.sftp-path-form input')?.value !== "/home/test/中 folder"; attempt++) await new Promise((resolve) => setTimeout(resolve, 50));
    if (document.querySelector<HTMLInputElement>('.sftp-path-form input')?.value !== "/home/test/中 folder") errors.push("Parsed OSC 7 did not update its directory");
    if (cwdCallbacks !== 1 || acknowledgedBytes !== writtenBytes) errors.push("OSC parsing or binary output ACK regressed");
  }
  if (query.has("aliases") || query.has("alias-error")) {
    const aliases = document.querySelector<HTMLDetailsElement>(".sftp-cwd-aliases");
    if (aliases) aliases.open = true;
    await settle();
    if (query.has("alias-error")) {
      const field = aliases?.querySelector<HTMLInputElement>("input");
      if (field) {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(field, "user@host");
        field.dispatchEvent(new Event("input", { bubbles: true }));
      }
      await settle();
      aliases?.querySelector<HTMLFormElement>("form")?.requestSubmit();
      await settle();
      const message = document.querySelector(".sftp-controls [role='alert']")?.textContent;
      if (!message?.includes(locale === "zh-CN" ? "主机名或 IP 地址" : "hostnames or IP addresses")) errors.push("Alias format error lacks host-specific recovery");
    }
    const input = aliases?.querySelector<HTMLInputElement>("input");
    input?.scrollIntoView({ block: "nearest" });
    await settle();
    const bounds = input?.getBoundingClientRect(), controls = document.querySelector(".sftp-controls")?.getBoundingClientRect();
    if (!bounds || !controls || bounds.top < controls.top || bounds.bottom > controls.bottom + 1) errors.push("Host alias input is clipped");
  }
  let fileRow: DOMRect | undefined, downloadEnabled = false;
  if (!query.has("collapsed")) {
    const file = [...document.querySelectorAll<HTMLButtonElement>(".sftp-entry-name")].find((entry) => entry.textContent === "中文 archive.tar.gz");
    const table = document.querySelector(".sftp-table-wrap")?.getBoundingClientRect();
    fileRow = file?.closest("tr")?.getBoundingClientRect();
    if (!fileRow || !table || fileRow.top < table.top || fileRow.bottom > table.bottom + 1 || fileRow.height < 24) errors.push("Selectable file row is clipped");
    file?.click(); await settle();
    const download = [...document.querySelectorAll<HTMLButtonElement>(".sftp-mutation-toolbar button")].find((button) => button.textContent === (locale === "zh-CN" ? "下载文件" : "Download file"));
    downloadEnabled = !!download && !download.disabled;
    if (!downloadEnabled) errors.push("Selecting a file did not enable Download");
  }
  if (query.has("dialog")) {
    const button = document.querySelector<HTMLButtonElement>(".transfer-row-actions button"); button?.click(); await settle();
    if (query.has("batch")) {
      const checkbox = document.querySelector<HTMLInputElement>('.transfer-batch-choice input');
      if (!checkbox || checkbox.checked) errors.push("Folder batch must require explicit consent");
      checkbox?.click(); await settle();
      const action = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((button) => button.textContent === (locale === "zh-CN" ? "覆盖本批次冲突" : "Replace batch conflicts"));
      if (!checkbox?.checked || !action || action.disabled) errors.push("Folder batch action missing");
      action?.scrollIntoView({ block: "nearest" }); await settle();
      const bounds = action?.getBoundingClientRect();
      if (!bounds || bounds.left < 0 || bounds.right > innerWidth || bounds.bottom > innerHeight) errors.push("Folder batch action clipped");
    }
  }
  if (query.has("cleanup-recovery")) {
    const recovery = document.querySelector(".transfer-recovery");
    if (!recovery?.textContent?.includes(locale === "zh-CN" ? "需要清理，无法续传" : "Cleanup required; cannot resume")) errors.push("Cleanup-only recovery state missing");
    const actions = [...(recovery?.querySelectorAll<HTMLButtonElement>(".transfer-row-actions button") ?? [])];
    if (actions.length !== 1 || actions[0].textContent !== (locale === "zh-CN" ? "删除残留" : "Remove partial")) errors.push("Cleanup-only recovery offers an incorrect action");
    actions[0]?.scrollIntoView({ block: "nearest" });
    const bounds = actions[0]?.getBoundingClientRect();
    if (!bounds || bounds.left < 0 || bounds.right > innerWidth || bounds.bottom > innerHeight) errors.push("Cleanup action cannot be reached within the viewport");
  }
  if (query.has("upload-recovery")) {
    const recovery = document.querySelector(".transfer-recovery");
    const actions = [...(recovery?.querySelectorAll<HTMLButtonElement>(".transfer-row-actions button") ?? [])];
    if (actions.length !== 1 || actions[0].textContent !== (locale === "zh-CN" ? "继续上传" : "Resume upload")) errors.push("Resumable upload must offer only source-verified continuation");
    const explanation = recovery?.querySelector(".transfer-row-progress p:last-child");
    if (!explanation?.textContent?.includes(locale === "zh-CN" ? "无法核验残留归属" : "Ownership cannot be verified")) errors.push("Upload cleanup ownership explanation missing");
    explanation?.scrollIntoView({ block: "nearest" });
    await settle();
    const bounds = explanation?.getBoundingClientRect();
    if (!bounds || bounds.left < 0 || bounds.right > innerWidth || bounds.bottom > innerHeight) errors.push("Upload recovery explanation cannot be reached within the viewport");
  }
  if (query.has("manual-recovery")) {
    const recovery = document.querySelector(".transfer-recovery");
    if (!recovery?.textContent?.includes(locale === "zh-CN" ? "需人工检查" : "Manual inspection required")) errors.push("Manual recovery state missing");
    if (recovery?.querySelector(".transfer-row-actions button")) errors.push("Unverifiable recovery must not offer mutation actions");
    const explanation = recovery?.querySelector(".transfer-row-progress p:last-child");
    explanation?.scrollIntoView({ block: "nearest" }); await settle();
    const bounds = explanation?.getBoundingClientRect();
    if (!bounds || bounds.left < 0 || bounds.right > innerWidth || bounds.bottom > innerHeight) errors.push("Manual recovery explanation cannot be reached within the viewport");
  }
  if (query.has("editor-dialog")) {
    for (let attempt = 0; attempt < 60 && !document.querySelector(".remote-edit-queue .transfer-row-actions button"); attempt++) await new Promise((resolve) => setTimeout(resolve, 50));
    const button = [...document.querySelectorAll<HTMLButtonElement>(".remote-edit-queue .transfer-row-actions button")].find((button) => button.textContent === (locale === "zh-CN" ? "处理冲突" : "Review conflict"));
    button?.click(); await settle();
    if (!document.querySelector('[role="dialog"] input')) errors.push("Editor conflict input missing");
  }
  if (query.has("editor-cleanup")) {
    const details = document.querySelector<HTMLDetailsElement>(".remote-edit-queue details");
    if (!details) errors.push("Remote temporary details missing");
    else { details.open = true; await settle(); }
    if (!details?.textContent?.includes(".capstan-edit-example.partial")) errors.push("Retained temporary path missing");
    const cleanup = [...document.querySelectorAll<HTMLButtonElement>(".remote-edit-queue button")].find((button) => button.textContent === (locale === "zh-CN" ? "重试远程清理" : "Retry remote cleanup"));
    if (!cleanup || cleanup.disabled) errors.push("Remote cleanup action missing");
  }
  if (query.has("editor-end-dialog")) {
    for (let attempt = 0; attempt < 60 && !document.querySelector(".remote-edit-queue .transfer-row"); attempt++) await new Promise((resolve) => setTimeout(resolve, 50));
    const buttons = [...document.querySelectorAll<HTMLButtonElement>(".remote-edit-queue button")];
    buttons.find((button) => button.textContent === (locale === "zh-CN" ? "结束编辑…" : "End editing…"))?.click();
    await settle();
    const retain = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((button) => button.textContent === (locale === "zh-CN" ? "保留文件供下次恢复" : "Keep files for next start"));
    if (!retain || retain.disabled) errors.push("Retain recovery files action missing");
    if (retain && retain.getBoundingClientRect().right > innerWidth) errors.push("Retain action exceeds viewport");
  }
  if (query.has("preferences-exit")) {
    emitPreferenceExit?.(); await settle();
    const text = document.querySelector('[role="dialog"]')?.textContent;
    if (!text?.includes(locale === "zh-CN" ? "面板偏好尚未保存完成" : "Panel preferences did not finish saving")) errors.push("Preference exit recovery missing");
  }
  if (query.has("native-export")) {
    const heading = [...document.querySelectorAll<HTMLButtonElement>(".transfer-queue-heading")].find((button) => button.textContent?.startsWith(locale === "zh-CN" ? "资源管理器导出" : "Explorer exports"));
    heading?.click(); await settle();
    const cleanup = [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === (locale === "zh-CN" ? "删除导出副本" : "Remove export copy"));
    cleanup?.click(); await settle();
    if (!document.querySelector('[role="dialog"]')?.textContent?.includes(locale === "zh-CN" ? "资源管理器完成复制" : "Explorer to finish copying")) errors.push("Export retention consent missing");
  }
  if (query.has("tree-plan")) {
    const action = [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')].find((button) => button.textContent?.startsWith(locale === "zh-CN" ? "合并已列出的目录" : "Merge listed directories"));
    action?.scrollIntoView({ block: "nearest" }); await settle();
    const bounds = action?.getBoundingClientRect();
    if (!bounds || bounds.left < 0 || bounds.right > innerWidth || bounds.bottom > innerHeight) errors.push("Reviewed directory action clipped");
  }
  if (query.has("delete-tree")) {
    // Recursive delete is an alert dialog; it shares the confirmation look.
    const apply = [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')].find((button) => button.textContent === (locale === "zh-CN" ? "删除此目录树" : "Delete this tree"));
    if (query.has("delete-result")) { apply?.click(); await settle(); }
    const action = query.has("delete-result") ? [...document.querySelectorAll<HTMLButtonElement>('[role="alertdialog"] button')].find((button) => button.textContent === (locale === "zh-CN" ? "关闭" : "Close")) : apply;
    action?.scrollIntoView({ block: "nearest" }); await settle();
    const bounds = action?.getBoundingClientRect();
    if (!bounds || bounds.left < 0 || bounds.right > innerWidth || bounds.bottom > innerHeight) errors.push("Recursive delete action clipped");
    if (query.has("delete-result") && document.querySelectorAll('.transfer-tree-directories li').length !== 3) errors.push("Recursive delete results missing");
  }
  const queue = document.querySelector(".transfer-queue")?.getBoundingClientRect();
  const panel = document.querySelector(".sftp-panel, .sftp-panel-collapsed")?.getBoundingClientRect();
  if (!queue || !panel) errors.push("Missing file workflow surface");
  if (queue && (queue.bottom > innerHeight + 1 || queue.width > innerWidth + 1)) errors.push("Queue exceeds viewport");
  if (panel && (panel.bottom > innerHeight + 1 || panel.right > innerWidth + 1)) errors.push("Panel exceeds viewport");
  const editor = document.querySelector(".remote-edit-queue")?.getBoundingClientRect();
  if (query.has("editor") && (!editor || editor.bottom > innerHeight + 1 || editor.right > innerWidth + 1)) errors.push("Editor queue exceeds viewport");
  if (document.documentElement.scrollWidth > innerWidth + 1) errors.push("Horizontal overflow");
  if (!query.has("collapsed") && document.querySelectorAll(".sftp-entry-name").length !== 3) errors.push("File rows missing");
  const dialog = document.querySelector('[role="dialog"], [role="alertdialog"]')?.getBoundingClientRect();
  if ((query.has("dialog") || query.has("preferences-exit") || query.has("editor-dialog") || query.has("editor-end-dialog") || query.has("tree-plan") || query.has("native-export") || query.has("delete-tree")) && (!dialog || dialog.bottom > innerHeight || dialog.right > innerWidth)) errors.push("File workflow dialog exceeds viewport");
  const name = [locale, theme, query.get("dock") ?? "right", query.has("dialog") ? "conflict" : "files", `${innerWidth}x${innerHeight}`].join("-");
  document.getElementById("result")!.dataset.report = encodeURIComponent(JSON.stringify({ passed: errors.length === 0, results: [{ name, passed: errors.length === 0, errors, queue, panel, dialog, fileRow, downloadEnabled, cwdCallbacks, acknowledgedBytes, writtenBytes }] }));
}
void check();
