export type TemplateVariable = { name: string; defaultValue: string | null };

/** Only `{{name}}` and `{{name:default}}` are variables. Any other double-brace
 * text, such as `docker ps --format '{{.Names}}'`, is sent as written. */
const placeholder = /\{\{([\p{ID_Start}_][\p{ID_Continue}]*)(?::([^{}]*))?\}\}/gu;

export function templateVariables(template: string): TemplateVariable[] {
  const variables = new Map<string, TemplateVariable>();
  for (const match of template.matchAll(placeholder)) {
    const name = match[1];
    if (!variables.has(name)) variables.set(name, { name, defaultValue: match[2] ?? null });
  }
  return [...variables.values()];
}

export function renderSnippetTemplate(template: string, values: Readonly<Record<string, string>>): string {
  let missing = false;
  const rendered = template.replace(placeholder, (_match, name: string) => {
    const value = Object.hasOwn(values, name) ? values[name] : undefined;
    if (value === undefined) {
      missing = true;
      return "";
    }
    return value;
  });
  if (missing) throw new Error("snippet_variable_missing");
  return rendered;
}

export function withAutoEnter(input: string, autoEnter: boolean): string {
  return autoEnter && !/[\r\n]$/u.test(input) ? `${input}\r` : input;
}
