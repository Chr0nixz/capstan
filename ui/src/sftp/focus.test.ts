import { describe, expect, it } from "vitest";
import { filesFocusTarget } from "./focus";

function panel(selected: boolean): HTMLElement {
  const root = document.createElement("div");
  root.innerHTML = `<div class="sftp-path-field"><input aria-label="Path"></div>
    <table class="sftp-table"><tbody>
      <tr><td><button class="sftp-entry-name">releases</button></td></tr>
      <tr class="${selected ? "selected" : ""}"><td><button class="sftp-entry-name">config.yaml</button></td></tr>
    </tbody></table>`;
  return root;
}

describe("remote files focus target", () => {
  it("prefers the selected row over the path field that precedes the table", () => {
    expect(filesFocusTarget(panel(true))?.textContent).toBe("config.yaml");
  });

  it("falls back to the first row, then the path field, then the collapsed strip", () => {
    expect(filesFocusTarget(panel(false))?.textContent).toBe("releases");
    const empty = document.createElement("div");
    empty.innerHTML = `<div class="sftp-path-field"><input aria-label="Path"></div>`;
    expect(filesFocusTarget(empty)?.getAttribute("aria-label")).toBe("Path");
    const collapsed = document.createElement("div");
    collapsed.innerHTML = `<button class="sftp-panel-collapsed">Remote files</button>`;
    expect(filesFocusTarget(collapsed)?.textContent).toBe("Remote files");
    expect(filesFocusTarget(null)).toBeNull();
  });
});
