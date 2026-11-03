// Development-only preview of the full workspace. It replaces the Tauri bridge
// with in-memory fixtures so the UI can be reviewed in a plain browser:
//   pnpm -C ui dev, then open /tests/app-preview.html
// Query options: ?locale=zh-CN  ?locked  ?ssh  ?prod (open a production host)  ?ended (SSH exits)  ?light  ?trust=new|changed
// No native, network, credential, or filesystem capability is reachable here.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "../src/App";
import { useWorkspace } from "../src/workspace";
import type { HostId } from "../src/terminal/contracts";
import "@xterm/xterm/css/xterm.css";
import "../src/styles.css";

const query = new URLSearchParams(location.search);
const requestedLocale = query.get("locale");
if (requestedLocale) Object.defineProperty(navigator, "language", { value: requestedLocale, configurable: true });

type Channel<T> = { onmessage: ((message: T) => void) | null };
class FixtureChannel<T> implements Channel<T> { onmessage: ((message: T) => void) | null = null; }

const hosts = [
  { id: "6f1d2c9a-1b5e-4c7d-9a10-000000000001", name: "build-01", address: "10.0.4.21", port: 22, username: "deploy", group: "g1", favorite: true, tags: ["ci"] },
  { id: "6f1d2c9a-1b5e-4c7d-9a10-000000000002", name: "prod-api-east", address: "api-east.internal", port: 22, username: "ops", group: "g2", favorite: true, tags: ["prod"] },
  { id: "6f1d2c9a-1b5e-4c7d-9a10-000000000003", name: "prod-db-primary", address: "db1.internal", port: 2222, username: "postgres", group: "g2", favorite: false, tags: ["prod", "db"] },
  { id: "6f1d2c9a-1b5e-4c7d-9a10-000000000004", name: "staging-web", address: "staging.example.test", port: 22, username: "www", group: "g1", favorite: false, tags: [] },
  { id: "6f1d2c9a-1b5e-4c7d-9a10-000000000005", name: "Raspberry Pi 家庭网关", address: "192.168.1.2", port: 22, username: "pi", group: null, favorite: false, tags: ["home"] },
].map((host) => ({
  id: host.id, name: host.name, address: host.address, port: host.port, username: host.username,
  credential_id: "cred-1", group_id: host.group, tags: host.tags, favorite: host.favorite, revision: 0,
  working_directory: null, startup_command: null, encoding: "utf-8", theme: query.has("light") && host.id.endsWith("1") ? "light" : "graphite", agent_forwarding: false, production: false,
}));
const groups = [{ id: "g1", name: "Build", parent_id: null, production: false }, { id: "g2", name: "Production", parent_id: null, production: true }];
const credentials = [
  { id: "cred-1", name: "deploy key", revision: 0, kind: "imported_key" },
  { id: "cred-2", name: "ops password", revision: 0, kind: "password" },
  { id: "cred-3", name: "Work agent", revision: 0, kind: "agent" },
];
const trusted = [
  { target: "10.0.4.21", port: 22, algorithm: "ssh-ed25519", fingerprint: "SHA256:q2Bf3pV0tq8m4wK2Yw1o0sN6l9cZ7xJ+e5RkT1uHn0Q", first_trusted_at: 1_760_000_000 },
  { target: "api-east.internal", port: 22, algorithm: "ecdsa-sha2-nistp256", fingerprint: "SHA256:7Lr0m1Q9cZ2vX3bN4kT8pW5yE6uR1oI2aS3dF4gH5jK", first_trusted_at: 1_761_200_000 },
];
const snippetGroups = [{ id: "6f1d2c9a-1b5e-4c7d-9a10-0000000000b1", name: "Deploy" }];
const snippets = [
  { id: "6f1d2c9a-1b5e-4c7d-9a10-0000000000c1", group_id: snippetGroups[0].id, name: "Tail app log", template: "tail -f /var/log/app.log", auto_enter: true },
  { id: "6f1d2c9a-1b5e-4c7d-9a10-0000000000c2", group_id: snippetGroups[0].id, name: "Restart service", template: "sudo systemctl restart {{service}}", auto_enter: false },
  { id: "6f1d2c9a-1b5e-4c7d-9a10-0000000000c3", group_id: null, name: "Disk usage", template: "df -h", auto_enter: true },
];
const sftpSession = "6f1d2c9a-1b5e-4c7d-9a10-0000000000a1";
const metrics = { queued_bytes: 0, in_flight_bytes: 0, max_outstanding_bytes: 0, input_bytes: 0, output_bytes: 0, frame_count: 0,
  max_pending_bytes: 0, max_in_flight_bytes: 0, pause_count: 0, resume_count: 0, canceled: false, channel_closed: false };

function checksum(bytes: Uint8Array): number {
  let hash = 0x811c9dc5;
  for (const byte of bytes) hash = Math.imul(hash ^ byte, 0x01000193) >>> 0;
  return hash;
}
type Session = { output: Channel<ArrayBuffer>; sequence: bigint; prompt: string };
const sessions = new Map<string, Session>();
function emit(id: string, text: string): void {
  const session = sessions.get(id);
  if (!session) return;
  const payload = new TextEncoder().encode(text);
  const buffer = new ArrayBuffer(20 + payload.length);
  const bytes = new Uint8Array(buffer), view = new DataView(buffer);
  bytes.set([0x43, 0x53, 0x54, 0x4e]);
  view.setBigUint64(4, session.sequence++, true);
  view.setUint32(12, payload.length, true);
  view.setUint32(16, checksum(payload), true);
  bytes.set(payload, 20);
  session.output.onmessage?.(buffer);
}
let sessionCount = 0;
function startSession(kind: "local" | "ssh", args: Record<string, unknown>, hostId?: string): Promise<unknown> {
  const id = `fixture-terminal-${++sessionCount}`;
  const output = args.onOutput as Channel<ArrayBuffer>, status = args.onStatus as Channel<unknown>;
  const host = hosts.find((candidate) => candidate.id === hostId);
  const prompt = kind === "ssh"
    ? `\x1b]7;file://${host?.name}/home/${host?.username}\x07\x1b[1;32m${host?.username}@${host?.name}\x1b[0m:\x1b[1;34m~\x1b[0m$ `
    : "PS C:\\Users\\dev> ";
  sessions.set(id, { output, sequence: 0n, prompt });
  const banner = kind === "ssh"
    ? `Welcome to Ubuntu 24.04.1 LTS (GNU/Linux 6.8.0-45-generic x86_64)\r\n\r\n  System load:  0.08     Processes:   142\r\n  Memory usage: 23%      Users logged in: 1\r\n\r\nLast login: Thu Oct 22 21:14:03 2026 from 10.0.4.1\r\n${prompt}ls -la\r\ntotal 48\r\ndrwxr-x--- 6 ${host?.username} ${host?.username} 4096 Oct 22 21:14 \x1b[1;34m.\x1b[0m\r\n-rw-r--r-- 1 ${host?.username} ${host?.username}  807 Oct 20 09:12 .profile\r\ndrwxrwxr-x 3 ${host?.username} ${host?.username} 4096 Oct 21 18:40 \x1b[1;34mreleases\x1b[0m\r\n-rwxr-xr-x 1 ${host?.username} ${host?.username} 2310 Oct 22 08:02 \x1b[1;32mdeploy.sh\x1b[0m\r\n-rw-r--r-- 1 ${host?.username} ${host?.username}  12K Oct 22 21:10 app.log\r\n${prompt}`
    : `Windows PowerShell\r\nCopyright (C) Microsoft Corporation. All rights reserved.\r\n\r\nInstall the latest PowerShell for new features and improvements! https://aka.ms/PSWindows\r\n\r\n${prompt}`;
  return new Promise((resolve) => window.setTimeout(() => {
    resolve(kind === "ssh" ? { kind, id, hostId, connectionId: "6f1d2c9a-1b5e-4c7d-9a10-0000000000c1" }
      : { kind, id, runtime: { source: "Bundled", path: null, version: "1.22", architecture: "x64", reason: "fixture" } });
    window.setTimeout(() => { emit(id, banner); status.onmessage?.({ id, state: "running", exit_code: null, error_code: null, metrics }); }, 30);
    if (kind === "ssh" && query.has("ended")) window.setTimeout(() => {
      emit(id, `logout\r\nConnection to ${host?.address ?? "host"} closed.\r\n`);
      status.onmessage?.({ id, state: "exited", exit_code: 0, error_code: null, metrics });
    }, 900);
  }, kind === "ssh" ? 1600 : 120));
}

const listeners = new Map<string, (event: { payload: unknown }) => void>();
const started = Date.now();
const directory = (path: string) => ({
  sessionId: sftpSession, requestRevision: "1", pageSequence: 0, path, hasMore: false, omittedEntries: 0,
  entries: [
    ["releases", "directory", null, 16877], ["logs", "directory", null, 16877], [".cache", "directory", null, 16877],
    ["deploy.sh", "file", "2310", 33261], ["app.log", "file", "12288", 33188], ["config.yaml", "file", "1874", 33188],
    ["build-2026-10-22.tar.gz", "file", "48318976", 33188], ["current", "symlink", null, 41471], [".profile", "file", "807", 33188],
  ].map(([name, kind, size, permissions], index) => ({
    id: `6f1d2c9a-1b5e-4c7d-9a10-0000000001${String(index).padStart(2, "0")}`, name,
    metadata: { kind, size, modifiedSeconds: String(1_761_100_000 + index * 3600), permissions },
  })),
});
let listed: ReturnType<typeof directory> | undefined;
const transfers = () => {
  const elapsed = (Date.now() - started) / 1000;
  const total = 48_318_976, processed = Math.min(total, Math.floor(total * ((elapsed % 40) / 40)));
  return [
    { id: "6f1d2c9a-1b5e-4c7d-9a10-0000000000d1", hostId: hosts[0].id, name: "build-2026-10-22.tar.gz", remotePath: "/home/deploy/releases/build-2026-10-22.tar.gz",
      direction: "upload", state: "running", version: "1", processed: String(processed), total: String(total), bytesPerSecond: "1258291", remainingSeconds: String(Math.max(0, Math.round((total - processed) / 1_258_291))),
      error: null, conflict: null, cleanupRequired: false, batch: null },
    { id: "6f1d2c9a-1b5e-4c7d-9a10-0000000000d2", hostId: hosts[1].id, name: "access.log.1", remotePath: "/var/log/nginx/access.log.1",
      direction: "download", state: "paused", version: "1", processed: "3145728", total: "9437184", bytesPerSecond: "0", remainingSeconds: null,
      error: null, conflict: null, cleanupRequired: false, batch: null },
  ];
};

window.__TAURI__ = {
  event: { listen: async <T,>(name: string, callback: (event: { payload: T }) => void) => {
    const listener = callback as (event: { payload: unknown }) => void;
    listeners.set(name, listener);
    return () => { if (listeners.get(name) === listener) listeners.delete(name); };
  } },
  core: {
    Channel: FixtureChannel,
    invoke: async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
      const result = await (async (): Promise<unknown> => {
        switch (command) {
          case "vault_status": return { exists: true, unlocked: !query.has("locked"), revision: 3, epoch: 1 };
          case "list_hosts": return hosts;
          case "list_credentials": return credentials;
          case "list_groups": return groups;
          case "list_trusted_host_keys": return trusted;
          case "vault_preferences": return { auto_lock_minutes: 30, disconnect_on_lock: false };
          case "check_vault_auto_lock": return false;
          case "list_snippet_groups": return snippetGroups;
          case "list_snippets": return snippets;
          case "list_tunnels": case "tunnel_runtime_snapshots": return [];
          case "list_local_shells": return [{ id: "pwsh", name: "PowerShell 7", is_wsl: false }, { id: "powershell51", name: "Windows PowerShell", is_wsl: false }, { id: "wsl:Ubuntu", name: "WSL: Ubuntu", is_wsl: true }];
          case "start_local_terminal": return startSession("local", args ?? {});
          case "start_ssh_terminal": return startSession("ssh", args ?? {}, args?.hostId as string);
          case "write_terminal_input": {
            const id = args?.id as string, input = String(args?.input ?? "");
            const session = sessions.get(id);
            if (session) emit(id, input.replace(/\r/g, `\r\n${session.prompt}`).replace(/\x7f/g, "\b \b"));
            return undefined;
          }
          case "load_sftp_panel_preferences": return { layout: { dock: "right", size: 38, collapsed: false }, session: "6f1d2c9a-1b5e-4c7d-9a10-0000000000e1", sequence: "1" };
          case "attach_sftp": return { sessionId: sftpSession, hostId: hosts[0].id, connectionGeneration: "1", path: "/home/deploy", atomicReplace: true, fsync: true, requestBytes: 32768 };
          case "list_remote_directory": return (listed = directory(String(args?.path ?? "/home/deploy")));
          case "prepare_remote_mutation": {
            // Preview only: a confirmation for the selected row, never applied.
            const entry = listed?.entries.find((candidate) => candidate.id === args?.entry);
            return entry && listed ? { token: "preview", hostId: hosts[0].id, path: `${listed.path}/${entry.name}`, metadata: entry.metadata, mutation: args?.mutation } : undefined;
          }
          case "update_remote_cwd": {
            const uri = String(args?.uri ?? ""), path = decodeURIComponent(uri.replace(/^file:\/\/[^/]*/, "")) || null;
            return { sequence: String(args?.sequence ?? "0"), path, aliases: [] };
          }
          case "configure_remote_cwd_aliases": return { sequence: "0", path: "/home/deploy", aliases: Array.isArray(args?.aliases) ? args.aliases : [] };
          case "transfer_snapshot": return transfers();
          case "transfer_recovery_snapshot": return { entries: [], incomplete: false };
          case "native_export_snapshot": return [];
          case "remote_edit_snapshot": return { revision: "0", sessions: [], recoveryIncomplete: false };
          case "remote_editor_configuration": return null;
          default: return undefined;
        }
      })();
      return result as T;
    },
  },
} as unknown as NonNullable<Window["__TAURI__"]>;

const root = document.getElementById("root");
if (!root) throw new Error("Preview root element is missing");
createRoot(root).render(<StrictMode><App /></StrictMode>);
if (query.has("ssh")) {
  const target = query.has("prod") ? hosts[1] : hosts[0];
  window.setTimeout(() => useWorkspace.getState().addSshTerminal(target.id as HostId, target.name, target.theme === "light" ? "light" : "graphite"), 300);
}
const trust = query.get("trust");
if (trust) {
  window.setTimeout(() => listeners.get("ssh-trust-challenge")?.({ payload: {
    challengeId: "preview", attemptId: "preview", hostId: hosts[1].id, hostRevision: 0, target: "api-east.internal", port: 22,
    algorithm: "ssh-ed25519", publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPreviewOnlyPublicKeyMaterial0000000000000000",
    fingerprint: "SHA256:Zx8q1Lr0m1Q9cZ2vX3bN4kT8pW5yE6uR1oI2aS3dF4g",
    knownAlgorithm: trust === "changed" ? "ssh-ed25519" : null,
    knownFingerprint: trust === "changed" ? "SHA256:7Lr0m1Q9cZ2vX3bN4kT8pW5yE6uR1oI2aS3dF4gH5jK" : null,
    keyChanged: trust === "changed",
  } }), 900);
}
