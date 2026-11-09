/** Where "Go to remote files" lands, most specific first. */
const filesFocusOrder = [".sftp-table tr.selected .sftp-entry-name", ".sftp-entry-name", ".sftp-path-field input", ".sftp-panel-collapsed"];

/**
 * The element "Go to remote files" focuses inside a terminal panel. A selector
 * list would return the first match in document order (the path field, which
 * comes before the table), so each target is tried in priority order instead.
 */
export function filesFocusTarget(panel: ParentNode | null | undefined): HTMLElement | null {
  for (const selector of filesFocusOrder) {
    const element = panel?.querySelector<HTMLElement>(selector);
    if (element) return element;
  }
  return null;
}
