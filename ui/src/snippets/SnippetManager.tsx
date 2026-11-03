import { useEffect, useMemo, useRef, useState, type FormEvent, type KeyboardEventHandler, type ReactElement, type ReactNode, type Ref } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { copy } from "../i18n";
import { CloseIcon, FolderPlusIcon, PencilIcon, PlusIcon, SnippetIcon, TrashIcon, CheckIcon } from "../ui/icons";
import { decodeSnippet, decodeSnippetGroup, type Snippet, type SnippetGroup } from "./contracts";
import { renderSnippetTemplate, templateVariables, withAutoEnter } from "./template";
import { snippetMutationError } from "./errors";

type Labels = typeof copy.en | typeof copy["zh-CN"];
type Draft = { id: string; group_id: string | null; name: string; template: string; auto_enter: boolean };
export type SnippetRunRequest = { requestId: string; snippetId: string };

type Props = {
  labels: Labels;
  groups: readonly SnippetGroup[];
  snippets: readonly Snippet[];
  search: string;
  onSearch: (value: string) => void;
  onSaveGroup: (group: SnippetGroup) => Promise<void>;
  onDeleteGroup: (id: string) => Promise<void>;
  onSaveSnippet: (snippet: Snippet) => Promise<void>;
  onDeleteSnippet: (id: string) => Promise<void>;
  onRun: (input: string, target: "current" | "broadcast") => Promise<void>;
  broadcastTargetsAvailable: boolean;
  runRequest?: SnippetRunRequest;
  onRequestConsumed: (requestId: string) => void;
  messages?: ReactNode;
  onClose: () => void;
  panelRef?: Ref<HTMLElement>;
  onKeyDown?: KeyboardEventHandler<HTMLElement>;
};

const emptyDraft = (): Draft => ({ id: crypto.randomUUID(), group_id: null, name: "", template: "", auto_enter: true });

export function SnippetManager({ labels, groups, snippets, search, onSearch, onSaveGroup, onDeleteGroup, onSaveSnippet, onDeleteSnippet, onRun, broadcastTargetsAvailable, runRequest, onRequestConsumed, messages, onClose, panelRef, onKeyDown }: Props): ReactElement {
  const [draft, setDraft] = useState<Draft | null>(null);
  const [groupName, setGroupName] = useState("");
  const [groupDrafts, setGroupDrafts] = useState<Record<string, string>>({});
  const previousGroups = useRef(groups);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const [running, setRunning] = useState(false);
  const runningRef = useRef(false);
  const consumedRequest = useRef<string | undefined>(undefined);
  const [error, setError] = useState("");
  const [runTarget, setRunTarget] = useState<"current" | "broadcast">("current");
  const [runDraft, setRunDraft] = useState<{ snippet: Snippet; values: Record<string, string> } | null>(null);
  const normalized = search.trim().toLocaleLowerCase();
  const visible = useMemo(() => snippets.filter((snippet) => !normalized
    || [snippet.name, snippet.template, groups.find((group) => group.id === snippet.group_id)?.name ?? ""]
      .some((value) => value.toLocaleLowerCase().includes(normalized))), [snippets, groups, normalized]);

  useEffect(() => {
    const previousNames = new Map(previousGroups.current.map((group) => [group.id, group.name]));
    previousGroups.current = groups;
    setGroupDrafts((current) => Object.fromEntries(groups.map((group) => {
      const edited = current[group.id];
      const keepEdit = edited !== undefined && edited !== previousNames.get(group.id) && edited.trim() !== group.name;
      return [group.id, keepEdit ? edited : group.name];
    })));
  }, [groups]);

  const finishRun = async (snippet: Snippet, values: Readonly<Record<string, string>>): Promise<void> => {
    if (runningRef.current) return;
    let input: string;
    try {
      input = withAutoEnter(renderSnippetTemplate(snippet.template, values), snippet.auto_enter);
    } catch {
      setError(labels.snippetInvalid);
      return;
    }
    runningRef.current = true;
    setRunning(true);
    setRunDraft(null);
    try {
      await onRun(input, runTarget);
    } catch {
      setError(labels.operationFailed);
    } finally {
      runningRef.current = false;
      setRunning(false);
    }
  };

  const startRun = (snippet: Snippet): void => {
    setError("");
    const variables = templateVariables(snippet.template);
    if (variables.length === 0) { void finishRun(snippet, {}); return; }
    setRunDraft({
      snippet,
      values: Object.fromEntries(variables.map(({ name, defaultValue }) => [name, defaultValue ?? ""])),
    });
  };

  useEffect(() => {
    if (!runRequest || consumedRequest.current === runRequest.requestId) return;
    // Mark the request before any callback or terminal side effect. StrictMode
    // can replay this effect before the parent's acknowledgement is committed.
    consumedRequest.current = runRequest.requestId;
    const snippet = snippets.find((item) => item.id === runRequest.snippetId);
    onRequestConsumed(runRequest.requestId);
    if (snippet) startRun(snippet);
    // A request is consumed once when the manager opens for this action.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runRequest, snippets]);

  const mutate = async (operation: () => Promise<void>): Promise<void> => {
    // A disabled button alone cannot stop two events in the same React batch.
    if (savingRef.current) return;
    savingRef.current = true;
    setSaving(true);
    setError("");
    try { await operation(); }
    catch (cause) { setError(snippetMutationError(cause, labels)); }
    finally { savingRef.current = false; setSaving(false); }
  };

  const save = async (event: FormEvent): Promise<void> => {
    event.preventDefault();
    if (savingRef.current) return;
    if (!draft || !draft.name.trim() || !draft.template.trim()) {
      setError(labels.snippetInvalid);
      return;
    }
    let snippet: Snippet;
    try {
      snippet = decodeSnippet({ ...draft, name: draft.name.trim() }, new Set(groups.map((group) => group.id)));
    } catch { setError(labels.snippetInvalid); return; }
    await mutate(async () => {
      await onSaveSnippet(snippet);
      setDraft(null);
    });
  };

  const saveGroup = async (event: FormEvent): Promise<void> => {
    event.preventDefault();
    if (savingRef.current) return;
    const name = groupName.trim();
    let group: SnippetGroup;
    try { group = decodeSnippetGroup({ id: crypto.randomUUID(), name }); }
    catch { setError(labels.snippetGroupInvalid); return; }
    await mutate(async () => {
      await onSaveGroup(group);
      setGroupName("");
    });
  };

  const saveGroupName = async (group: SnippetGroup): Promise<void> => {
    if (savingRef.current) return;
    const name = groupDrafts[group.id]?.trim() ?? "";
    let updated: SnippetGroup;
    try { updated = decodeSnippetGroup({ ...group, name }); }
    catch { setError(labels.snippetGroupInvalid); return; }
    await mutate(() => onSaveGroup(updated));
  };

  return <aside ref={panelRef} className="sheet snippet-manager" role="dialog" aria-labelledby="snippet-manager-title" aria-busy={saving || running} tabIndex={-1} onKeyDown={onKeyDown}>
    <header className="sheet-header">
      <h2 id="snippet-manager-title"><SnippetIcon size={14} />{labels.snippets}</h2>
      <button className="icon-button" type="button" aria-label={labels.closePanel} title={labels.closePanel} onClick={onClose}><CloseIcon /></button>
    </header>
    {messages}
    <div className="sheet-body">
      <div className="snippet-toolbar">
        <input type="search" aria-label={labels.snippetSearch} placeholder={labels.snippetSearch} value={search} onChange={(event) => onSearch(event.currentTarget.value)} />
        <label className="snippet-target"><span>{labels.snippetTarget}</span><select value={runTarget} onChange={(event) => setRunTarget(event.currentTarget.value as "current" | "broadcast")}>
          <option value="current">{labels.snippetCurrentPane}</option>
          <option value="broadcast" disabled={!broadcastTargetsAvailable}>{labels.snippetBroadcastPanes}</option>
        </select></label>
        <button className="icon-button" type="button" aria-label={labels.newSnippet} title={labels.newSnippet} disabled={saving} onClick={() => { setError(""); setDraft(emptyDraft()); }}><PlusIcon /></button>
      </div>
      <form className="snippet-group-create" onSubmit={(event) => { void saveGroup(event); }}>
        <input aria-label={labels.newSnippetGroup} placeholder={labels.newSnippetGroup} value={groupName} disabled={saving} onChange={(event) => setGroupName(event.currentTarget.value)} maxLength={128} />
        <button className="icon-button" type="submit" disabled={saving || !groupName.trim()} aria-label={labels.saveSnippetGroup} title={labels.saveSnippetGroup}><FolderPlusIcon /></button>
      </form>
      {groups.length > 0 && <ul className="item-list snippet-group-list" aria-label={labels.snippetGroup}>
        {groups.map((group) => <li key={group.id}><input aria-label={`${labels.snippetGroup}: ${group.name}`} value={groupDrafts[group.id] ?? group.name} disabled={saving} onChange={(event) => {
          const value = event.currentTarget.value;
          setGroupDrafts((current) => ({ ...current, [group.id]: value }));
        }} maxLength={128} />
          <div className="item-actions"><button className="icon-button" type="button" aria-label={`${labels.saveSnippetGroup}: ${group.name}`} title={labels.saveSnippetGroup} disabled={saving || !(groupDrafts[group.id] ?? group.name).trim()}
            onClick={() => { void saveGroupName(group); }}><CheckIcon size={14} /></button><button className="icon-button" type="button" aria-label={`${labels.deleteSnippetGroup}: ${group.name}`} title={labels.deleteSnippetGroup}
            disabled={saving} onClick={() => { void mutate(() => onDeleteGroup(group.id)); }}><TrashIcon size={14} /></button></div></li>)}
      </ul>}
      {visible.length === 0 && <p className="empty-state">{snippets.length === 0 ? labels.noSnippets : labels.noSnippetMatches}</p>}
      <ul className="item-list snippet-list" aria-label={labels.snippets}>
        {visible.map((snippet) => <li key={snippet.id}>
          <button className="snippet-run" type="button" disabled={running} onClick={() => startRun(snippet)}><strong>{snippet.name}</strong><span>{groups.find((group) => group.id === snippet.group_id)?.name ?? labels.noGroup}</span></button>
          <div className="item-actions">
            <button className="icon-button" type="button" aria-label={`${labels.editSnippet}: ${snippet.name}`} title={labels.editSnippet} disabled={saving} onClick={() => setDraft({ ...snippet })}><PencilIcon size={14} /></button>
            <button className="icon-button" type="button" aria-label={`${labels.deleteSnippet}: ${snippet.name}`} title={labels.deleteSnippet} disabled={saving} onClick={() => { void mutate(() => onDeleteSnippet(snippet.id)); }}><TrashIcon size={14} /></button>
          </div>
        </li>)}
      </ul>
      {error && <p className="inline-error" role="alert">{error}</p>}
    </div>
    {/* Real dialogs: Esc closes only the editor (never the sheet and its
        draft), and focus stays inside until it closes. */}
    {draft && <Dialog.Root open onOpenChange={(open) => { if (!open && !savingRef.current) setDraft(null); }}><Dialog.Portal>
      <Dialog.Overlay className="snippet-editor-scrim" />
      <Dialog.Content asChild aria-describedby={undefined}>
        <form className="snippet-editor" onSubmit={(event) => { void save(event); }}>
          <Dialog.Title asChild><h3>{snippets.some((snippet) => snippet.id === draft.id) ? labels.editSnippet : labels.newSnippet}</h3></Dialog.Title>
          <label>{labels.snippetName}<input autoFocus maxLength={256} value={draft.name} disabled={saving} onChange={(event) => setDraft({ ...draft, name: event.currentTarget.value })} /></label>
          <label>{labels.snippetGroup}<select value={draft.group_id ?? ""} disabled={saving} onChange={(event) => setDraft({ ...draft, group_id: event.currentTarget.value || null })}>
            <option value="">{labels.noGroup}</option>{groups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
          </select></label>
          <label>{labels.snippetTemplate}<textarea rows={6} maxLength={8192} value={draft.template} disabled={saving} aria-describedby={`${draft.id}-template-hint`}
            spellCheck={false} onChange={(event) => setDraft({ ...draft, template: event.currentTarget.value })} />
            <span className="field-hint" id={`${draft.id}-template-hint`}>{labels.snippetTemplateHint}</span></label>
          <label className="check-row"><input type="checkbox" checked={draft.auto_enter} disabled={saving} onChange={(event) => setDraft({ ...draft, auto_enter: event.currentTarget.checked })} />{labels.snippetAutoEnter}</label>
          {error && <p className="inline-error" role="alert">{error}</p>}
          <div className="snippet-editor-actions">
            <button className="primary-action" type="submit" disabled={saving}>{labels.saveSnippet}</button>
            <button className="ghost-button" type="button" disabled={saving} onClick={() => setDraft(null)}>{labels.snippetCancelRun}</button>
          </div>
        </form>
      </Dialog.Content>
    </Dialog.Portal></Dialog.Root>}
    {runDraft && <Dialog.Root open onOpenChange={(open) => { if (!open) setRunDraft(null); }}><Dialog.Portal>
      <Dialog.Overlay className="snippet-editor-scrim" />
      <Dialog.Content asChild>
        <form className="snippet-editor snippet-variable-form" onSubmit={(event) => { event.preventDefault(); void finishRun(runDraft.snippet, runDraft.values); }}>
          <Dialog.Title asChild><h3>{labels.snippetVariables}</h3></Dialog.Title>
          <Dialog.Description>{runDraft.snippet.name}</Dialog.Description>
          {templateVariables(runDraft.snippet.template).map(({ name }) => <label key={name}>{labels.snippetVariable(name)}<input autoFocus={name === templateVariables(runDraft.snippet.template)[0]?.name} value={runDraft.values[name] ?? ""}
            onChange={(event) => {
              const value = event.currentTarget.value;
              setRunDraft((current) => current ? { ...current, values: { ...current.values, [name]: value } } : current);
            }} /></label>)}
          <div className="snippet-editor-actions">
            <button className="primary-action" type="submit">{labels.snippetRun}</button>
            <button className="ghost-button" type="button" onClick={() => setRunDraft(null)}>{labels.snippetCancelRun}</button>
          </div>
        </form>
      </Dialog.Content>
    </Dialog.Portal></Dialog.Root>}
  </aside>;
}


