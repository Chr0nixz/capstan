import { describe, expect, it } from "vitest";
import { benchmarkCopy } from "./benchmark/copy";
import { editorCopy } from "./editors/copy";
import { copy, detectLocale } from "./i18n";
import { pluralize } from "./plural";
import { sftpCopy } from "./sftp/copy";
import { treeDeleteCopy } from "./sftp/treeDeleteCopy";
import { transferCopy } from "./transfers/copy";

/** Flattens a dictionary into key paths: functions keep their arity and arrays their full structure. */
function shape(value: unknown, path = "", out = new Map<string, string>()): Map<string, string> {
  if (typeof value === "function") out.set(path, `function/${value.length}`);
  else if (Array.isArray(value)) {
    out.set(path, `array/${value.length}`);
    value.forEach((item, index) => { if (typeof item === "object" && item !== null) shape(item, `${path}[${index}]`, out); });
  } else if (typeof value === "object" && value !== null) {
    for (const [key, item] of Object.entries(value)) shape(item, path ? `${path}.${key}` : key, out);
  } else out.set(path, typeof value);
  return out;
}

const dictionaries = { copy, benchmarkCopy, editorCopy, sftpCopy, transferCopy, treeDeleteCopy };

describe("i18n dictionaries", () => {
  it.each(Object.entries(dictionaries))("%s has the same keys, arities and lists in en and zh-CN", (_, dictionary) => {
    expect(Object.fromEntries(shape(dictionary["zh-CN"]))).toEqual(Object.fromEntries(shape(dictionary.en)));
  });

  it("maps every Chinese WebView language to zh-CN and everything else to en", () => {
    expect(["zh-CN", "zh", "zh-TW", "zh-Hans-CN", "ZH-cn"].map((language) => detectLocale(language))).toEqual(Array(5).fill("zh-CN"));
    expect(["en-US", "ja-JP", "fr", ""].map((language) => detectLocale(language))).toEqual(Array(4).fill("en"));
  });

  it("uses singular English nouns for a count of one", () => {
    expect(pluralize(1, "file", "files")).toBe("1 file");
    expect(pluralize(0, "file", "files")).toBe("0 files");
    expect(treeDeleteCopy.en.summary(1, 1, 1)).toBe("1 file · 1 directory · 1 symbolic link");
    expect(sftpCopy.en.previewSummary(1, 2)).toBe("Adds 1 line and removes 2 lines.");
    expect(transferCopy.en.batch("logs", 1)).toBe("Folder batch: logs · 1 file");
    expect(transferCopy.en.batchScope(1)).toBe("Apply to the existing destination in this folder batch");
    expect(copy.en.tunnelTraffic(1, "1 B", "2 B")).toBe("1 connection · Sent 1 B · Received 2 B");
    expect(copy.en.confirmMultilinePaste(1)).toBe("Paste 1 line into the remote terminal?");
    expect(copy.en.confirmOsc52Write(1)).toBe("Allow the remote host to replace the clipboard with 1 character?");
    expect(copy.en.backupPreviewCounts(1, 1, 1, 1, 1, 1)).toBe("Backup contains 1 host, 1 group, 1 credential, 1 host key, 1 snippet group, and 1 snippet.");
  });
});
