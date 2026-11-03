import { invokeTauri } from "../terminal/bridge";
import type { TerminalSessionId } from "../terminal/contracts";
import type { DirectoryPage, RemoteEntryId, SftpSessionId } from "../sftp/contracts";
import { decodeSelection } from "./contracts";

export async function selectAndEnqueue(
  terminal: TerminalSessionId, session: SftpSessionId, page: DirectoryPage, locale: string,
  entry: RemoteEntryId | null, current: () => boolean,
): Promise<void> {
  const selection = decodeSelection(await (entry
    ? invokeTauri("select_transfer_destination", { terminal, session, entry, locale })
    : invokeTauri("select_transfer_source", { terminal, session, parent: page.path, locale })));
  if (!selection) return;
  let consumed = false;
  try {
    if (!current()) return;
    await invokeTauri("enqueue_transfer", { terminal, session, selection: selection.token,
      target: entry ? { kind: "download", entry } : { kind: "upload", parent: page.path, name: selection.name } });
    consumed = true;
  } finally {
    if (!consumed) await invokeTauri("release_transfer_selection", { token: selection.token }).catch(() => undefined);
  }
}

/**
 * Select a fresh upload source for an interrupted task. The returned token is
 * consumed by the retry command; the local path never crosses the IPC boundary.
 */
export async function selectSourceForRetry(
  terminal: TerminalSessionId, session: SftpSessionId, parent: string, locale: string,
): Promise<string | null> {
  const selection = decodeSelection(await invokeTauri("select_transfer_source", {
    terminal, session, parent, locale,
  }));
  return selection?.token ?? null;
}

export async function selectRecoveredDestination(terminal: TerminalSessionId, session: SftpSessionId, id: string, locale: string): Promise<string | null> {
  return decodeSelection(await invokeTauri("select_recovered_transfer_destination", { terminal, session, id, locale }))?.token ?? null;
}

