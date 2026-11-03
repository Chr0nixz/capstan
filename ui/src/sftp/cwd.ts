import { createStore } from "zustand/vanilla";
import type { TerminalSessionId } from "../terminal/contracts";
import { invokeTauri } from "../terminal/bridge";
import { decodeExactU64, validUtf8, type AttachedSftp, type ExactU64, type SftpErrorCode } from "./contracts";
import { type CwdHint, cwdHints } from "./cwdHints";
import { createSftpController, sftpError } from "./controller";

export type CwdSnapshot = { sequence: ExactU64; path: string | null; aliases: string[] };
export interface CwdTransport {
  update(attached: AttachedSftp, hint: CwdHint): Promise<CwdSnapshot>;
  aliases(attached: AttachedSftp, aliases: string[] | null): Promise<CwdSnapshot>;
}

export function decodeCwdSnapshot(value: unknown): CwdSnapshot {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("sftp_contract_invalid");
  const data = value as Record<string, unknown>;
  if (Object.keys(data).length !== 3 || !["sequence", "path", "aliases"].every((key) => Object.hasOwn(data, key))
    || (data.path !== null && (typeof data.path !== "string" || !data.path.startsWith("/")
      || !validUtf8(data.path, 4096) || /[\u0000-\u001f\u007f-\u009f]/u.test(data.path)))
    || !Array.isArray(data.aliases) || data.aliases.length > 8
    || !data.aliases.every((alias: unknown) => typeof alias === "string" && alias.length <= 253 && /^[a-zA-Z0-9.:[\]-]+$/.test(alias))) {
    throw new Error("sftp_contract_invalid");
  }
  return { sequence: decodeExactU64(data.sequence), path: data.path as string | null, aliases: data.aliases as string[] };
}

export function tauriCwdTransport(terminal: TerminalSessionId): CwdTransport {
  return {
    update: async (attached, hint) => decodeCwdSnapshot(await invokeTauri("update_remote_cwd", {
      terminal, session: attached.sessionId, generation: attached.connectionGeneration, ...hint,
    })),
    aliases: async (attached, aliases) => decodeCwdSnapshot(await invokeTauri("configure_remote_cwd_aliases", {
      terminal, session: attached.sessionId, generation: attached.connectionGeneration, aliases,
    })),
  };
}

/** Each browser owns its hint/navigation state, including hidden tabs. */
export function createCwdController(
  terminal: TerminalSessionId,
  browser: ReturnType<typeof createSftpController>,
  transport: CwdTransport = tauriCwdTransport(terminal),
  hub: Pick<typeof cwdHints, "subscribe"> = cwdHints,
) {
  const store = createStore<{ follow: boolean; snapshot: CwdSnapshot | null; error: SftpErrorCode | null; errorSource: "aliases" | "hint" | null; saving: boolean }>(() => ({
    follow: true, snapshot: null, error: null, errorSource: null, saving: false,
  }));
  let disposed = true, active = false, blocked = false, processing = false;
  let attachment: AttachedSftp | null = null;
  let epoch = 0;
  let latest: CwdHint | null = null, pending: CwdHint | null = null;
  let attempted = "";
  let aliasError: SftpErrorCode | null = null;
  const valid = (operation: number, session: AttachedSftp): boolean => !disposed && epoch === operation && attachment === session;
  const navigate = (): void => {
    const state = browser.store.getState(), cwd = store.getState(), snapshot = cwd.snapshot;
    if (disposed || !active || blocked || processing || pending || !cwd.follow || state.busy
      || !snapshot?.path || !state.page || !attachment || cwd.saving) return;
    const key = `${attachment.sessionId}:${snapshot.sequence}`;
    if (attempted === key) return;
    attempted = key;
    if (state.page.path !== snapshot.path) void browser.navigate(snapshot.path);
  };
  const accept = (snapshot: CwdSnapshot): void => {
    store.setState({ snapshot, error: aliasError, errorSource: aliasError ? "aliases" : null });
  };
  const flush = async (): Promise<void> => {
    if (processing || disposed || !attachment || store.getState().saving) return;
    processing = true;
    try {
      while (pending && attachment && !disposed && !store.getState().saving) {
        const hint = pending, session = attachment, operation = epoch;
        pending = null;
        try {
          const snapshot = await transport.update(session, hint);
          if (valid(operation, session)) accept(snapshot);
        } catch (cause) {
          if (valid(operation, session)) store.setState({ error: sftpError(cause), errorSource: "hint" });
        }
      }
    } finally { processing = false; navigate(); }
  };
  const browserChanged = (state: ReturnType<typeof browser.store.getState>): void => {
    if (state.attached !== attachment) {
      attachment = state.attached; epoch++; attempted = ""; pending = latest;
      aliasError = null;
      store.setState({ snapshot: null, error: null, errorSource: null, saving: false });
      void flush();
    }
    navigate();
  };
  let unsubscribeBrowser = (): void => undefined;
  let unsubscribeHints = (): void => undefined;
  const aliases = async (values: string[] | null): Promise<boolean> => {
    const session = attachment;
    if (!session || disposed || store.getState().saving) return false;
    const operation = ++epoch;
    aliasError = null;
    store.setState({ saving: true, error: null, errorSource: null });
    try {
      const snapshot = await transport.aliases(session, values);
      if (!valid(operation, session)) return false;
      accept(snapshot); attempted = "";
      return true;
    } catch (cause) {
      if (valid(operation, session)) { aliasError = sftpError(cause); store.setState({ error: aliasError, errorSource: "aliases" }); }
      return false;
    } finally {
      if (valid(operation, session)) { pending = latest; store.setState({ saving: false }); void flush(); navigate(); }
    }
  };
  return {
    store,
    start(): void {
      if (!disposed) return;
      disposed = false;
      unsubscribeBrowser = browser.store.subscribe(browserChanged);
      browserChanged(browser.store.getState());
      unsubscribeHints = hub.subscribe(terminal, (hint) => {
        latest = hint; pending = hint;
        if (!hint) { epoch++; attempted = ""; store.setState({ snapshot: null }); }
        if (hint) void flush();
      });
    },
    setActivity(visible: boolean, busy: boolean): void { active = visible; blocked = busy; navigate(); },
    setFollow(follow: boolean): void { store.setState({ follow }); attempted = ""; navigate(); },
    aliases,
    dispose(): void { if (!disposed) { disposed = true; epoch++; unsubscribeBrowser(); unsubscribeHints(); } },
  };
}
