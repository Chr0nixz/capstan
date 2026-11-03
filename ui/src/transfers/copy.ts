import type { TransferState } from "./contracts";
type Copy = {
  title: string; upload: string; uploadFolder: string; download: string; downloadFolder: string; cancel: string; pause: string; resume: string; retry: string; replace: string; rename: string; skip: string; name: string;
  empty: string; noReplace: string; typeConflict: string; cleanup: string; conflict: string; changed: string; retryConnection: string; openConnection: string; exitTitle: string; exitDescription: string; keepOpen: string; exit: string;
  eta: (seconds: string) => string; states: Record<TransferState, string>;
  batch: (name: string, files: number) => string; batchScope: (count: number) => string; batchHelp: string;
  replaceBatch: string; skipBatch: string;
  residuals: string; recoveryIncomplete: string; manualRecovery: string; recoveryState: Record<"awaiting_reconnect" | "available" | "missing" | "changed" | "manual_inspection" | "cleanup_required", string>;
  resumeRecovered: string; resumeDownload: string; localPartial: string; downloadRecoveryManual: string; uploadRecoveryManual: string;
  removePartial: string; removePartialTitle: string; removePartialDescription: string; removePartialConfirm: string;
  unknownHost: string; clearFinished: string; summaryActive: (count: number, eta: string | null) => string; summaryPaused: (count: number) => string; summaryAttention: (count: number) => string;
  existingSize: (size: string) => string; incomingSize: (size: string) => string;
  exports: string; prepareExport: string; dragExport: string; dropHint: string; clearExport: string; exportRetention: string; exportCleanup: string; clearExportConfirm: string;
  exportStates: Record<"planning" | "preparing" | "ready" | "dragging" | "retained" | "failed", string>;
  treeTitle: string; treeHelp: string; skipSubtree: string; mergeDirectory: string; mergeDirectories: string;
  treeKinds: Record<"directory_exists" | "type_conflict" | "invalid_name" | "name_collision", string>;
};
export const transferCopy: Record<"en" | "zh-CN", Copy> = {
  en: {
    treeTitle: "Review folder destinations", treeHelp: "Resolve folder and filename conflicts before copying. Skipping a directory skips its contents. Changed destinations require another review; existing files keep their own replacement decisions.", skipSubtree: "Skip this item", mergeDirectory: "Merge this directory", mergeDirectories: "Merge listed directories",
    treeKinds: { directory_exists: "This directory already exists.", type_conflict: "The destination has a different file type. Rename or skip it.", invalid_name: "This name cannot be used at the destination. Choose another name or skip it.", name_collision: "Two source names map to the same destination. Rename or skip one." },
    unknownHost: "SSH host", clearFinished: "Clear finished",
    summaryActive: (count, eta) => `${count} in progress${eta ? ` · ${eta}` : ""}`, summaryPaused: (count) => `${count} paused`, summaryAttention: (count) => `${count} need${count === 1 ? "s" : ""} a decision`,
    existingSize: (size) => `Existing destination: ${size}`, incomingSize: (size) => `This transfer: ${size}`,
    exports: "Explorer exports", prepareExport: "Drag out to Explorer…", dragExport: "Drag to Explorer", dropHint: "Drop files or folders here to upload to the displayed directory.", clearExport: "Remove export copy", exportRetention: "Export copies stay available until you confirm Explorer has finished copying.", exportCleanup: "Wait for Explorer to finish copying before removing this staging copy. Any unfinished export downloads will be canceled.", clearExportConfirm: "Copy finished — remove",
    exportStates: { planning: "Review export names", preparing: "Preparing export", ready: "Ready to drag", dragging: "Dragging", retained: "Retained copy", failed: "Export incomplete" },
    typeConflict: "The destination has a different file type. Save with another name or skip; existing directories are preserved.",
    title: "Transfers", upload: "Upload file", uploadFolder: "Upload folder", download: "Download file", downloadFolder: "Download folder", cancel: "Cancel transfer", pause: "Pause", resume: "Resume", retry: "Retry", replace: "Replace", rename: "Save as", skip: "Skip", name: "New filename",
    empty: "No transfers yet.", noReplace: "This server cannot safely replace an existing file. Save with another name or skip.", retryConnection: "Reconnect the SSH session for this host, then retry the transfer.", openConnection: "Open SSH session",
    cleanup: "A temporary file may remain or the commit result may be uncertain. Inspect the destination before retrying.",
    conflict: "Review destination", changed: "The destination exists or changed. Review this version before replacing it.",
    exitTitle: "File operations are still active", exitDescription: "Keep Capstan open to continue, or cancel remaining transfers and directory deletions, then exit. Current removals and final commits finish before exit. Completed deletions cannot be undone.", keepOpen: "Keep open", exit: "Cancel remaining operations and exit",
    eta: (seconds) => `${seconds}s remaining`,
    batch: (name, files) => `Folder batch: ${name} · ${files} files`, batchScope: (count) => `Apply to the ${count} existing destinations in this folder batch`,
    batchHelp: "Only the versions checked for this batch are included. Changed destinations need another decision. Save as applies to this file.",
    replaceBatch: "Replace batch conflicts", skipBatch: "Skip batch conflicts",
    residuals: "Interrupted transfer files", recoveryIncomplete: "Some recovery records could not be read. Unverified files were left untouched.",
    recoveryState: { awaiting_reconnect: "Reconnect to verify", available: "Local partial found", missing: "Partial is missing", changed: "Partial changed", manual_inspection: "Manual inspection required", cleanup_required: "Cleanup required; cannot resume" },
    manualRecovery: "The partial's ownership or original authorization cannot be verified. Keep the partial and inspect it manually.",
    resumeRecovered: "Resume upload", resumeDownload: "Resume download", localPartial: "A local partial was retained.", downloadRecoveryManual: "Choose the original download destination again to verify and resume this partial.",
    uploadRecoveryManual: "Choose the original source to verify and resume. Ownership cannot be verified for removal; inspect the remote partial manually before deleting it.",
    removePartial: "Remove partial", removePartialTitle: "Remove interrupted transfer data?", removePartialDescription: "The retained partial bytes will be permanently deleted. Local downloads require selecting the original destination again. The final file is left unchanged.", removePartialConfirm: "Remove partial",
    states: { queued: "Queued", preparing: "Preparing", running: "Transferring", paused: "Paused", resuming: "Resuming", retrying: "Retrying", verifying: "Verifying", committing: "Committing", completed: "Completed", awaiting_conflict: "Decision needed", failed: "Failed", canceled: "Canceled", interrupted: "Interrupted" },
  },
  "zh-CN": {
    treeTitle: "检查文件夹目标", treeHelp: "复制前请处理目录与名称冲突。跳过目录会跳过其内容；目标变化后需重新确认，同名文件仍需单独决定是否覆盖。", skipSubtree: "跳过此项", mergeDirectory: "合并此目录", mergeDirectories: "合并已列出的目录",
    treeKinds: { directory_exists: "此目录已存在。", type_conflict: "目标类型不同，请改名或跳过。", invalid_name: "目标位置不支持此名称，请改名或跳过。", name_collision: "两个来源名称对应同一目标，请改名或跳过其中一项。" },
    unknownHost: "SSH 主机", clearFinished: "清除已完成",
    summaryActive: (count, eta) => `${count} 个进行中${eta ? ` · ${eta}` : ""}`, summaryPaused: (count) => `${count} 个已暂停`, summaryAttention: (count) => `${count} 个需要处理`,
    existingSize: (size) => `目标现有文件：${size}`, incomingSize: (size) => `本次传输：${size}`,
    exports: "资源管理器导出", prepareExport: "拖出到资源管理器…", dragExport: "拖到资源管理器", dropHint: "将文件或文件夹拖到此处，上传到当前显示的目录。", clearExport: "删除导出副本", exportRetention: "导出副本会保留，直到你确认资源管理器已完成复制。", exportCleanup: "请等待资源管理器完成复制后，再删除此暂存副本；未完成的导出下载将取消。", clearExportConfirm: "复制已完成，删除副本",
    exportStates: { planning: "检查导出名称", preparing: "正在准备导出", ready: "可以拖出", dragging: "正在拖拽", retained: "已保留副本", failed: "导出未完成" },
    typeConflict: "目标类型不同。请另存为或跳过，已有目录将保留。",
    title: "传输任务", upload: "上传文件", uploadFolder: "上传文件夹", download: "下载文件", downloadFolder: "下载文件夹", cancel: "取消传输", pause: "暂停", resume: "继续", retry: "重试", replace: "覆盖", rename: "另存为", skip: "跳过", name: "新文件名",
    empty: "暂无传输任务。", noReplace: "服务器不支持安全覆盖现有文件。请另存为其他名称或跳过。", retryConnection: "请先重新连接此主机的 SSH 会话，再重试传输。", openConnection: "打开 SSH 会话",
    cleanup: "目标目录可能保留临时文件，或最终提交结果不确定。重试前请检查目标。", conflict: "检查传输目标", changed: "目标已存在或发生变化。覆盖前请重新确认此版本。",
    exitTitle: "仍有文件操作未完成", exitDescription: "保持 Capstan 打开以继续，或取消剩余传输和目录删除后退出。当前删除请求和最终提交会在退出前完成；已经删除的条目无法撤销。", keepOpen: "保持打开", exit: "取消剩余操作并退出",
    eta: (seconds) => `预计剩余 ${seconds} 秒`,
    batch: (name, files) => `文件夹批次：${name} · ${files} 个文件`, batchScope: (count) => `应用于此文件夹批次中预检到的 ${count} 个同名目标`,
    batchHelp: "仅适用于本批次已检查的版本。目标再次变化时需重新决定；另存为仅作用于当前文件。",
    replaceBatch: "覆盖本批次冲突", skipBatch: "跳过本批次冲突",
    residuals: "中断传输残留", recoveryIncomplete: "部分恢复记录无法读取，未核实的文件均保持原样。",
    recoveryState: { awaiting_reconnect: "重新连接后核验", available: "发现本地残留", missing: "残留文件缺失", changed: "残留文件已变化", manual_inspection: "需人工检查", cleanup_required: "需要清理，无法续传" },
    manualRecovery: "无法核验残留文件的归属或原始授权。请保留残留并人工检查。",
    resumeRecovered: "继续上传", resumeDownload: "继续下载", localPartial: "已保留本地部分文件。", downloadRecoveryManual: "重新选择原下载目标后，将校验并续传此残留。",
    uploadRecoveryManual: "重新选择原文件后可校验并续传。暂无法核验残留归属，请先人工检查远端残留；此处不提供自动删除。",
    removePartial: "删除残留", removePartialTitle: "删除中断的传输数据？", removePartialDescription: "已保留的部分数据将被永久删除；本地下载需重新选择原目标以核验，最终目标文件保持不变。", removePartialConfirm: "删除残留",
    states: { queued: "排队中", preparing: "准备中", running: "传输中", paused: "已暂停", resuming: "恢复中", retrying: "重试中", verifying: "校验中", committing: "提交中", completed: "已完成", awaiting_conflict: "等待决策", failed: "失败", canceled: "已取消", interrupted: "已中断" },
  },
};
