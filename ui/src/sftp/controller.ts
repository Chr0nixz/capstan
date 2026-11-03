import { createStore } from "zustand/vanilla";
import { sftpErrorCodes, type AttachedSftp, type Confirmation, type DirectoryPage, type Mutation, type RemoteEditView, type RemoteEntryId, type Shell, type ShellAction, type ShellPreview, type SftpErrorCode } from "./contracts";
import type { SftpTransport } from "./transport";

export type BrowserState = {
  attached: AttachedSftp | null; page: DirectoryPage | null;
  busy: boolean; error: SftpErrorCode | null; selected: RemoteEntryId | null;
};
export function sftpError(cause: unknown): SftpErrorCode {
  const code: unknown = typeof cause === "object" && cause !== null && "code" in cause ? cause.code : null;
  return sftpErrorCodes.find((value) => value === code) ?? "remote_io";
}

// Keep at most 8 pages (2048 entry capabilities), below the backend's 4096
// entry budget. Older pages can be revisited by re-enumerating the directory.
const retainedPages = 8;
export function createSftpController(transport: SftpTransport) {
  const store = createStore<BrowserState>(() => ({ attached: null, page: null, busy: false, error: null, selected: null }));
  let generation = 0;
  let disposed = false;
  let mutationRunning = false;
  let pages: DirectoryPage[] = [];
  let committedPage: DirectoryPage | null = null;
  let navigationTail: Promise<void> = Promise.resolve();
  const current = (operation: number): boolean => !disposed && operation === generation;
  const fail = (cause: unknown, operation: number): void => {
    if (current(operation)) store.setState({ busy: false, error: sftpError(cause) });
  };
  const remember = async (page: DirectoryPage, operation: number): Promise<void> => {
    pages.push(page);
    store.setState({ page, selected: null });
    if (pages.length > retainedPages) {
      const removed = pages.shift();
      if (removed && current(operation)) await transport.release(page.sessionId, removed.entries.map((entry) => entry.id));
    }
  };
  const validatePage = (page: DirectoryPage, attached: AttachedSftp, previous?: DirectoryPage): void => {
    if (page.sessionId !== attached.sessionId || page.pageSequence !== (previous ? previous.pageSequence + 1 : 0)
      || (previous && (page.requestRevision !== previous.requestRevision || page.path !== previous.path))) {
      pages = [];
      committedPage = null;
      store.setState({ page: null, selected: null });
      throw { code: "scope_expired" };
    }
  };
  const navigate = async (path: string, targetPage = 0): Promise<void> => {
    const attached = store.getState().attached;
    if (!attached || disposed || mutationRunning) return;
    const operation = ++generation;
    store.setState({ busy: true, error: null, selected: null });
    // Serialize directory requests so an older IPC cannot replace the server
    // scope after the newest page has been displayed.
    const load = async (): Promise<void> => {
      if (!current(operation)) return;
      try {
        let page = await transport.list(attached.sessionId, path);
        if (!disposed && store.getState().attached?.sessionId === attached.sessionId
          && page.sessionId === attached.sessionId && page.pageSequence === 0) {
          committedPage = page;
        }
        if (!current(operation)) return;
        validatePage(page, attached);
        pages = [];
        await remember(page, operation);
        while (current(operation) && page.pageSequence < targetPage && page.hasMore) {
          const nextPage = await transport.next(attached.sessionId, page.requestRevision);
          if (!current(operation)) return;
          validatePage(nextPage, attached, page);
          page = nextPage;
          await remember(page, operation);
        }
        if (current(operation)) store.setState({ busy: false });
      } catch (cause) {
        // An earlier in-flight navigation may have committed on the server
        // before this request failed. Recover that scope, not stale entry IDs.
        if (current(operation) && committedPage && committedPage.requestRevision !== store.getState().page?.requestRevision) {
          pages = [committedPage];
          store.setState({ page: committedPage, selected: null });
        }
        fail(cause, operation);
      }
    };
    navigationTail = navigationTail.then(load, load);
    await navigationTail;
  };
  const start = async (): Promise<void> => {
    if (mutationRunning) return;
    disposed = false;
    const operation = ++generation;
    const previous = store.getState().attached;
    pages = [];
    committedPage = null;
    store.setState({ busy: true, error: null, attached: null, page: null, selected: null });
    try {
      if (previous) await transport.detach(previous.sessionId).catch(() => undefined);
      if (!current(operation)) return;
      const attached = await transport.attach();
      if (!current(operation)) { await transport.detach(attached.sessionId).catch(() => undefined); return; }
      store.setState({ attached });
      await navigate(attached.path);
    } catch (cause) { fail(cause, operation); }
  };
  const next = async (): Promise<void> => {
    const { attached, page, busy } = store.getState();
    if (!attached || !page?.hasMore || busy || disposed) return;
    const cached = pages.find((entry) => entry.pageSequence === page.pageSequence + 1);
    if (cached) { store.setState({ page: cached, selected: null, error: null }); return; }
    const operation = ++generation;
    store.setState({ busy: true, error: null, selected: null });
    try {
      const nextPage = await transport.next(attached.sessionId, page.requestRevision);
      if (!current(operation)) return;
      validatePage(nextPage, attached, page);
      await remember(nextPage, operation);
      if (current(operation)) store.setState({ busy: false });
    } catch (cause) { fail(cause, operation); }
  };
  const previous = async (): Promise<void> => {
    const { page, busy } = store.getState();
    if (!page || page.pageSequence === 0 || busy || disposed) return;
    const cached = pages.find((entry) => entry.pageSequence === page.pageSequence - 1);
    if (cached) { store.setState({ page: cached, selected: null, error: null }); return; }
    await navigate(page.path, page.pageSequence - 1);
  };
  const mutate = async (action: (attached: AttachedSftp, page: DirectoryPage) => Promise<void>): Promise<boolean> => {
    const { attached, page, busy } = store.getState();
    if (!attached || !page || busy || disposed) return false;
    const operation = ++generation;
    mutationRunning = true;
    store.setState({ busy: true, error: null });
    try {
      await action(attached, page);
      if (!current(operation)) return false;
      mutationRunning = false;
      await navigate(page.path);
      // A successful mutation must not be repeated when refreshing fails.
      return true;
    } catch (cause) { fail(cause, operation); return false; }
    finally { mutationRunning = false; }
  };
  return {
    store, start, navigate, next, previous,
    select: (selected: RemoteEntryId | null): void => {
      if (!disposed && !store.getState().busy) store.setState({ selected });
    },
    create: (name: string, directory: boolean): Promise<boolean> => mutate((attached, page) => transport.create(attached.sessionId, page.path, name, directory)),
    previewShell: (shell: Shell, action: ShellAction, entry?: RemoteEntryId): Promise<ShellPreview> => {
      const { attached, selected, busy } = store.getState();
      const target = entry ?? selected;
      if (!attached || !target || busy || disposed) return Promise.reject({ code: "scope_expired" });
      return transport.previewShell?.(attached.sessionId, target, shell, action) ?? Promise.reject({ code: "operation_canceled" });
    },
    applyShell: (entry: RemoteEntryId, token: string): Promise<unknown> => {
      const { attached, busy } = store.getState();
      if (!attached || busy || disposed) return Promise.reject({ code: "scope_expired" });
      return transport.applyShell?.(attached.sessionId, entry, token) ?? Promise.reject({ code: "operation_canceled" });
    },
    selectEditor: (locale: string): Promise<string | null> => transport.selectEditor?.(locale) ?? Promise.reject({ code: "editor_unavailable" }),
    editorConfiguration: (): Promise<string | null> => transport.editorConfiguration?.() ?? Promise.resolve(null),
    beginEditor: (entry: RemoteEntryId, editorId: string): Promise<RemoteEditView> => {
      const { attached, busy } = store.getState();
      if (!attached || busy || disposed) return Promise.reject({ code: "scope_expired" });
      return transport.beginEditor?.(attached.sessionId, entry, editorId) ?? Promise.reject({ code: "editor_unavailable" });
    },
    prepare: async (mutation: Mutation): Promise<Confirmation | null> => {
      const { attached, selected, busy } = store.getState();
      if (!attached || !selected || busy || disposed) return null;
      const operation = ++generation;
      mutationRunning = true;
      store.setState({ busy: true, error: null });
      try {
        const confirmation = await transport.prepare(attached.sessionId, selected, mutation);
        if (!current(operation)) return null;
        store.setState({ busy: false });
        return confirmation;
      } catch (cause) { fail(cause, operation); return null; }
      finally { mutationRunning = false; }
    },
    apply: (confirmation: Confirmation): Promise<boolean> => mutate((attached) => transport.apply(attached.sessionId, confirmation.token)),
    dispose: async (): Promise<void> => {
      if (disposed) return;
      disposed = true; ++generation;
      const attached = store.getState().attached;
      pages = [];
      committedPage = null;
      store.setState({ attached: null, page: null, busy: false, selected: null });
      if (attached) await transport.detach(attached.sessionId).catch(() => undefined);
    },
  };
}
