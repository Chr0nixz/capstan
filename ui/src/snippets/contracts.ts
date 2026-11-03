export type SnippetId = string & { readonly snippet: unique symbol };
export type SnippetGroupId = string & { readonly snippetGroup: unique symbol };
export type SnippetGroup = { id: SnippetGroupId; name: string };
export type Snippet = {
  id: SnippetId; group_id: SnippetGroupId | null; name: string; template: string; auto_enter: boolean;
};

const uuidPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/iu;
const nilId = "00000000-0000-0000-0000-000000000000";
const utf8 = new TextEncoder();
// Match Rust str::trim (Unicode White_Space), which differs from JavaScript trim.
const nonWhitespace = /[^\u0009-\u000d\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]/u;
const invalid = (): never => { throw new Error("snippet_contract_invalid"); };

function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return invalid();
  const result = value as Record<string, unknown>;
  if (Object.keys(result).length !== keys.length || !keys.every((key) => Object.hasOwn(result, key))) return invalid();
  return result;
}

function validText(value: unknown, maximum: number, required: boolean): value is string {
  // Limits are UTF-8 bytes; lone surrogates cannot be decoded as Rust strings.
  return typeof value === "string" && !/[\ud800-\udfff]/u.test(value)
    && utf8.encode(value).byteLength <= maximum && (!required || nonWhitespace.test(value))
    && !value.includes("\0");
}

export function decodeSnippetGroup(value: unknown): SnippetGroup {
  const group = record(value, ["id", "name"]);
  if (typeof group.id !== "string" || !uuidPattern.test(group.id) || group.id === nilId || !validText(group.name, 128, true)
    || /[\u0000-\u001f\u007f-\u009f]/u.test(group.name)) return invalid();
  return { id: group.id as SnippetGroupId, name: group.name };
}

export function decodeSnippet(value: unknown, groupIds: ReadonlySet<string>): Snippet {
  const snippet = record(value, ["id", "group_id", "name", "template", "auto_enter"]);
  if (typeof snippet.id !== "string" || !uuidPattern.test(snippet.id) || snippet.id === nilId
    || (snippet.group_id !== null && (typeof snippet.group_id !== "string" || !uuidPattern.test(snippet.group_id)
      || snippet.group_id === nilId || !groupIds.has(snippet.group_id)))
    || !validText(snippet.name, 256, true) || /[\u0000-\u001f\u007f-\u009f]/u.test(snippet.name)
    || !validText(snippet.template, 8192, true) || typeof snippet.auto_enter !== "boolean") return invalid();
  return {
    id: snippet.id as SnippetId,
    group_id: snippet.group_id as SnippetGroupId | null,
    name: snippet.name,
    template: snippet.template,
    auto_enter: snippet.auto_enter,
  };
}
