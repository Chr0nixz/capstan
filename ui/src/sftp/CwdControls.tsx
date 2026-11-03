import { useEffect, useId, useState, useSyncExternalStore, type ReactElement } from "react";
import type { createCwdController } from "./cwd";
import { sftpCopy } from "./copy";
import { ChevronRightIcon } from "../ui/icons";

type CwdController = ReturnType<typeof createCwdController>;

/** Follow sits at the end of the path row; it is the quiet default when on. */
export function FollowToggle({ controller, locale, ended = false }: {
  controller: CwdController; locale: keyof typeof sftpCopy; ended?: boolean;
}): ReactElement {
  const state = useSyncExternalStore(controller.store.subscribe, controller.store.getState, controller.store.getState);
  const labels = sftpCopy[locale];
  // Following is the quiet default; amber only marks a follow that is still waiting for a hint.
  return <button className="follow-toggle" type="button" aria-pressed={state.follow} aria-label={labels.follow}
    data-waiting={state.follow && !state.snapshot?.path && !ended ? "true" : undefined} data-ended={ended || undefined}
    title={labels.followHint} onClick={() => controller.setFollow(!state.follow)}><span className="follow-dot" aria-hidden="true" />{labels.followShort}</button>;
}

/**
 * The waiting status and the server-hostname editor. They only take a row while
 * Follow is waiting for a hint, the editor is open, or it reports an error;
 * the More menu opens the editor at any time.
 */
export function CwdControls({ controller, locale, ended = false, aliasesOpen, onAliasesOpenChange }: {
  controller: CwdController; locale: keyof typeof sftpCopy; ended?: boolean;
  aliasesOpen: boolean; onAliasesOpenChange: (open: boolean) => void;
}): ReactElement | null {
  const state = useSyncExternalStore(controller.store.subscribe, controller.store.getState, controller.store.getState);
  const labels = sftpCopy[locale];
  const inputId = useId();
  const [aliases, setAliases] = useState("");
  const error = state.errorSource === "aliases" && state.error === "path_invalid" ? labels.cwdAliasesInvalid
    : state.errorSource === "aliases" && state.error === "resource_limit" ? labels.cwdAliasesLimit
      : state.error ? labels.errors[state.error] : null;
  useEffect(() => { if (state.snapshot) setAliases(state.snapshot.aliases.join(", ")); }, [state.snapshot?.aliases.join(",")]);
  useEffect(() => { if (aliasesOpen) void controller.aliases(null); }, [aliasesOpen, controller]);
  const waiting = state.follow && !state.snapshot?.path && !ended;
  const row = waiting || aliasesOpen || state.errorSource === "aliases";
  if (!row && !error) return null;
  return <>
    {row && <div className="sftp-cwd-row">
      {waiting && <span className="sftp-status" role="status">{labels.waitingCwd}</span>}
      <details className="sftp-cwd-aliases" open={aliasesOpen} onToggle={(event) => {
        if (event.currentTarget.open !== aliasesOpen) onAliasesOpenChange(event.currentTarget.open);
      }}>
        <summary title={labels.cwdAliasesHint}><ChevronRightIcon size={12} />{labels.cwdAliases}</summary>
        <p className="sftp-status">{labels.cwdAliasesHint}</p>
        <form className="sftp-path-form" onSubmit={(event) => {
          event.preventDefault();
          void controller.aliases(aliases.split(",").map((alias) => alias.trim()).filter(Boolean));
        }}>
          <label htmlFor={inputId}>{labels.cwdAliasNames}</label>
          <input id={inputId} value={aliases} onChange={(event) => setAliases(event.currentTarget.value)} maxLength={2048} autoComplete="off" spellCheck={false} />
          <button type="submit" disabled={state.saving}>{labels.cwdBindAliases}</button>
        </form>
      </details>
    </div>}
    {error && <p className="sftp-error" role="alert">{error}</p>}
  </>;
}
