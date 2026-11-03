import type { RemoteEditState, RemoteTemporaryState } from "../sftp/contracts";
type EditorCopy = {
  title: string; configure: string; recovered: string; unsent: string; saved: string; manage: string; close: string;
  closeHint: string; exportCopy: string; discard: string; cancel: string; retry: string; conflict: string; conflictHint: string;
  keepRemote: string; replace: string; saveAs: string; name: string; noReplace: string; recoveryIncomplete: string; discover: string;
  states: Record<RemoteEditState, string>;
  reconnect: string; reconnectCleanup: string; retain: string; recoveryError: string; temporaries: string; cleanup: string; cleanupHint: string; temporaryStates: Record<RemoteTemporaryState, string>;
};
export const editorCopy: Record<"en" | "zh-CN", EditorCopy> = {
  en: {
    retain: "Keep files for next start", recoveryError: "Recovery metadata could not be verified. Export the local copy or keep all files for the next start. Remote cleanup and deletion stay blocked until the metadata can be verified.",
    reconnectCleanup: "Connect this SSH host again. Only the recorded temporary files will be checked for cleanup. A recovered document will not be uploaded.",
    temporaries: "Retained remote temporary files", cleanup: "Retry remote cleanup",
    cleanupHint: "Only files matching the recorded metadata and full hash are removed. Changed or unverified files need remote inspection. Reconnect first if the session was interrupted. A successful local export may still leave this session open until remote cleanup succeeds.",
    temporaryStates: { unverified: "Ownership unverified", recorded: "Ownership recorded", changed: "Changed remotely" },
    title: "Remote editing", configure: "Choose editor", recovered: "Recovered local copy", unsent: "Local changes retained", saved: "Remote copy is up to date",
    manage: "Review conflict", close: "End editing…", closeHint: "Close this document in the external editor first. Export it, keep the working files for the next start, or explicitly discard it. Keeping files stops uploads and preserves the recovery folder. Canceling the folder picker keeps this session open.",
    exportCopy: "Export local copy…", discard: "Discard local copy", cancel: "Keep editing", retry: "Retry upload",
    conflict: "Resolve edit conflict", conflictHint: "The remote version and your local changes are retained. Choose a new remote name, review replacement, or discard your local changes. If the remote changes again, another decision is required.",
    keepRemote: "Keep remote and discard local", replace: "Replace remote", saveAs: "Save under remote name", name: "New remote filename",
    noReplace: "Atomic replacement is unavailable for this target. Save under a new name or export a local copy.",
    recoveryIncomplete: "Some editor folders could not be recovered. They remain on disk. Resolve the listed copies and scan again; unknown files and other running instances are preserved.", discover: "Scan for retained copies",
    states: { opening: "Opening", waiting_for_stable_save: "Watching saves", ready_to_upload: "Uploading", conflict: "Remote changed", residual: "Copy retained", closed: "Closed" },
    reconnect: "Connect this SSH host again. The requested retry will use its verified SFTP session and recheck the original remote baseline.",
  },
  "zh-CN": {
    retain: "保留文件供下次恢复", recoveryError: "恢复元数据未能通过核验。可以导出本地副本，或保留全部文件供下次启动恢复。元数据重新通过核验前，不会清理远端或删除恢复文件。",
    reconnectCleanup: "请重新连接此 SSH 主机。此次请求只核验并清理记录中的临时文件，不会回传恢复的文档。",
    temporaries: "保留的远程临时文件", cleanup: "重试远程清理",
    cleanupHint: "只有属性和完整哈希仍与记录一致的文件才会删除。已变化或无法核验的文件需要在远端检查；会话中断时请先重连。本地导出成功后，仍可能需要完成远程清理才能结束会话。",
    temporaryStates: { unverified: "所有权尚未核验", recorded: "已记录所有权基线", changed: "远端已修改" },
    title: "远程编辑", configure: "选择编辑器", recovered: "恢复的本地副本", unsent: "本地修改已保留", saved: "远端副本已同步",
    manage: "处理冲突", close: "结束编辑…", closeHint: "请先在外部编辑器中关闭此文档。可以导出、保留工作文件供下次恢复，或明确放弃。保留文件会停止回传并留下恢复目录；取消文件夹选择后，编辑会话仍会保留。",
    exportCopy: "导出本地副本…", discard: "放弃本地副本", cancel: "继续编辑", retry: "重试回传",
    conflict: "处理编辑冲突", conflictHint: "远端版本和本地修改均已保留。可以使用新的远程文件名、确认覆盖，或放弃本地修改。远端再次变化时，需要重新决定。",
    keepRemote: "保留远端并放弃本地", replace: "覆盖远端", saveAs: "以新文件名保存到远端", name: "新的远程文件名",
    noReplace: "当前目标无法原子覆盖，请使用新的文件名或导出本地副本。",
    recoveryIncomplete: "部分编辑目录暂时无法恢复，仍保留在磁盘上。处理列表中的副本后可重新扫描；未知文件及其他运行实例的目录不会被删除。", discover: "扫描保留的副本",
    states: { opening: "正在打开", waiting_for_stable_save: "监测保存", ready_to_upload: "正在回传", conflict: "远端已变化", residual: "副本已保留", closed: "已关闭" },
    reconnect: "请重新连接此 SSH 主机。此次重试会使用已验证的新 SFTP 会话，并重新校验原远端基线。",
  },
};
