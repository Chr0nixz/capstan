export const treeDeleteCopy = {
  en: {
    title: "Delete directory contents", scanning: "Checking the directory scope…", scope: "This removes the listed directory and its contents. Symbolic links are removed without following them. Changes require a new confirmation; completed removals cannot be undone.",
    summary: (files: number, directories: number, links: number) => `${files} files · ${directories} directories · ${links} symbolic links`,
    apply: "Delete this tree", deleting: "Deleting…", stop: "Cancel remaining", stopping: "Waiting for the current removal…",
    result: (removed: number, retained: number) => `${removed} removed · ${retained} failed or retained`,
    outcomes: { removed: "Removed", failed: "Failed", retained: "Retained" }, close: "Close", previous: "Previous results", next: "Next results",
  },
  "zh-CN": {
    title: "删除目录及内容", scanning: "正在检查目录范围…", scope: "将删除所列目录及其内容。符号链接只删除链接，不跟随目标。内容变化需要重新确认；已经删除的条目无法撤销。",
    summary: (files: number, directories: number, links: number) => `${files} 个文件 · ${directories} 个目录 · ${links} 个符号链接`,
    apply: "删除此目录树", deleting: "正在删除…", stop: "取消剩余删除", stopping: "正在等待当前删除结果…",
    result: (removed: number, retained: number) => `已删除 ${removed} 项 · 失败或保留 ${retained} 项`,
    outcomes: { removed: "已删除", failed: "失败", retained: "已保留" }, close: "关闭", previous: "上一页结果", next: "下一页结果",
  },
} as const;
