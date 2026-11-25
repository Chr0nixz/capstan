import type { ReactElement } from "react";

/**
 * Splits a name so its distinguishing end survives truncation: `prod-api-east`
 * keeps `-east`, `build · Pane 2` keeps `· Pane 2`. Names with no short last
 * part truncate at the end as usual.
 */
export function splitLabel(text: string): [string, string] {
  const pane = /^(.+?)(\s·\s.{1,16})$/u.exec(text);
  if (pane) return [pane[1], pane[2]];
  const part = /^(.+?)([-_.\s][^-_.\s]{1,8})$/u.exec(text);
  return part ? [part[1], part[2]] : [text, ""];
}

/** A one-line label whose start gives way first, so the end that tells names apart stays. */
export function MiddleTruncate({ text, className }: { text: string; className?: string }): ReactElement {
  const [head, tail] = splitLabel(text);
  return <span className={className ? `middle-truncate ${className}` : "middle-truncate"}>
    <span className="middle-truncate-head">{head}</span>{tail && <span className="middle-truncate-tail">{tail}</span>}
  </span>;
}
