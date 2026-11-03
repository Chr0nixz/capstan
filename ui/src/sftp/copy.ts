import type { FileKind, SftpErrorCode } from "./contracts";
type SftpCopy = {
  title: string; path: string; go: string; parent: string; refresh: string; hidden: string; copy: string;
  follow: string; followShort: string; followHint: string; cwdAliasesMenu: string; waitingCwd: string; cwdAliases: string; cwdAliasesHint: string; cwdAliasNames: string; cwdBindAliases: string;
  cwdAliasesInvalid: string; cwdAliasesLimit: string;
  preferencesLoadFailed: string; preferencesSaveFailed: string; preferencesRetry: string;
  preferencesExitTitle: string; preferencesExitDescription: string; preferencesRetryExit: string;
  newFile: string; newDirectory: string; rename: string; remove: string; permissions: string;
  edit: string; editorOpening: string; editorSaving: string; editorConflict: string; editorClose: string; editorKeepRemote: string; editorReplace: string; editorSaveAs: string; editorNamePrompt: string;
  shellIntegration: string; shellIntegrationTitle: string; shellIntegrationHint: string; shell: string; action: string; install: string; uninstall: string;
  preview: string; applyIntegration: string; previewLoading: string; previewSummary: (added: number, removed: number) => string; noChanges: string;
  name: string; size: string; type: string; modified: string; next: string; previous: string; retry: string;
  page: (number: number) => string; omitted: (count: number) => string;
  loading: string; empty: string; noVisible: string; chooseName: string; chooseMode: string;
  panelSize: string; createTitle: (directory: boolean) => string; cancel: string; apply: string;
  renameTarget: (name: string) => string; permissionChange: (before: string, after: string) => string; deleteScopeNote: string;
  deleteTitle: (name: string, host: string) => string; deleteBody: string; changeTitle: (name: string, host: string) => string; sessionEnded: string;
  collapse: string; expand: string; dock: string; right: string; bottom: string; copied: string;
  production: string; moreActions: string; dockRight: string; dockBottom: string; deleteKind: (kind: string) => string;
  kinds: Record<FileKind, string>; errors: Record<SftpErrorCode, string>;
};
export const sftpCopy: Record<"en" | "zh-CN", SftpCopy> = {
  en: {
    title: "Remote files", path: "Remote path", go: "Open", parent: "Parent", refresh: "Refresh", hidden: "Hidden files", copy: "Copy path",
    follow: "Follow terminal", followShort: "Follow", cwdAliasesMenu: "Server hostnames…", followHint: "Follow this terminal's directory hints. Opening a path manually turns follow off.", waitingCwd: "Waiting for shell directory hints",
    cwdAliases: "Server hostnames", cwdAliasesHint: "If this server reports its own name differently from the address you connect to, list up to 8 of its hostnames here, separated by commas, so Follow terminal accepts its directory hints. Names printed by the shell are never trusted automatically.", cwdAliasNames: "Hostnames", cwdBindAliases: "Save names",
    cwdAliasesInvalid: "Enter hostnames or IP addresses separated by commas. Remove user names, ports, paths and spaces inside a name, then save again.", cwdAliasesLimit: "List at most 8 hostnames, then save again.",
    preferencesLoadFailed: "Saved panel preferences could not be read. This session's layout remains available. Check settings.json format and permissions, then retry.", preferencesSaveFailed: "Panel preferences could not be saved. This layout is kept for this session. Check the app data folder's space and permissions, then retry.", preferencesRetry: "Retry preferences",
    preferencesExitTitle: "Panel preferences did not finish saving", preferencesExitDescription: "The window stays open while the file operation finishes. You can keep working or retry closing.", preferencesRetryExit: "Retry exit",
    newFile: "New file", newDirectory: "New directory", rename: "Rename", remove: "Delete", permissions: "Permissions",
    edit: "Edit", editorOpening: "Opening editor…", editorSaving: "Saving remote file…", editorConflict: "The remote file changed while you were editing.", editorClose: "Close editor", editorKeepRemote: "Keep remote", editorReplace: "Replace remote", editorSaveAs: "Save local as…", editorNamePrompt: "New remote filename",
    shellIntegration: "Shell integration", shellIntegrationTitle: "Shell integration", shellIntegrationHint: "Preview the managed block before changing the selected remote configuration.", shell: "Shell", action: "Action", install: "Install", uninstall: "Uninstall",
    preview: "Preview", applyIntegration: "Apply integration", previewLoading: "Reading the remote configuration…", previewSummary: (added, removed) => `Adds ${added} lines and removes ${removed} lines.`, noChanges: "No changes are needed.",
    name: "Name", size: "Size", type: "Type / mode", modified: "Modified", next: "Next page", previous: "Previous page", retry: "Reconnect SFTP",
    page: (number) => `Page ${number}`, omitted: (count) => `${count} unsupported entries omitted. Supported files remain available; inspect other names in the terminal.`,
    loading: "Loading remote directory…", empty: "This directory is empty. Create a file or directory to begin.", noVisible: "No visible entries on this page. Show hidden files or open the next page.",
    chooseName: "Name", chooseMode: "Permissions (octal)", panelSize: "Panel size",
    createTitle: (directory) => directory ? "Create directory" : "Create file", cancel: "Cancel", apply: "Confirm",
    renameTarget: (name) => `Rename to ${name}`,
    permissionChange: (before, after) => `Permissions ${before} → ${after}`,
    deleteScopeNote: "Directories must be empty to delete.",
    deleteTitle: (name, host) => `Delete ${name} from ${host}?`, deleteBody: "This can't be undone.",
    changeTitle: (name, host) => `Change ${name} on ${host}?`, sessionEnded: "The SSH session ended. Reconnect to browse files again.",
    collapse: "Collapse remote files", expand: "Show remote files", dock: "Panel position", right: "Right", bottom: "Bottom", copied: "Path copied.",
    production: "Production", moreActions: "More file actions", dockRight: "Move panel to the right", dockBottom: "Move panel to the bottom", deleteKind: (kind) => `Delete ${kind.toLocaleLowerCase()}`,
    kinds: { file: "File", directory: "Directory", symlink: "Link", other: "Special", unknown: "Unknown" },
    errors: {
      sftp_unavailable: "The server refused SFTP. Terminal sessions remain available. Check the server configuration, then reconnect SFTP.",
      permission_denied: "Permission denied. Check the remote account permissions, then refresh.", path_invalid: "Enter an absolute remote path or a valid single name.",
      not_found: "The remote path no longer exists. Check the path or refresh the directory.",
      unsupported_file_type: "This filename or file type is unsupported. No operation was applied. Use the terminal to inspect it.",
      target_conflict: "The target changed or already exists. Refresh and review the operation again.", resume_mismatch: "The saved transfer no longer matches. Review the source and target.",
      local_io: "The local file operation failed. Check available space and permissions.", remote_io: "The remote operation failed or its result is uncertain. Refresh before retrying a mutation.",
      editor_unavailable: "The configured editor is unavailable.", connection_lost: "SFTP disconnected. Reconnect the SSH session or retry SFTP.", operation_canceled: "The operation was canceled.",
      editor_file_too_large: "This file exceeds the 8 MiB editor limit. Use a file transfer instead.",
      editor_invalid_encoding: "Save as valid UTF-8 or BOM-marked UTF-16, then retry. The local copy is retained.",
      editor_unsupported_content: "This file contains unsupported text or control bytes. The local copy is retained.",
      editor_conflict: "The remote file changed. Review the conflict before uploading again.",
      editor_stale_save: "The local file is still changing. Wait for the editor to finish saving, then retry.",
      editor_residual: "Review the open or recovered editor copies before exiting. Close the external document first, then export a copy, keep the files for recovery, or explicitly discard it. Cleanup failures retain the session.",
      scope_expired: "This file view expired. Reconnect SFTP to obtain a current directory.", resource_limit: "The file service reached its limit. Close unused views or wait, then retry.",
    },
  },
  "zh-CN": {
    title: "远程文件", path: "远程路径", go: "打开", parent: "上级目录", refresh: "刷新", hidden: "隐藏文件", copy: "复制路径",
    follow: "跟随终端", followShort: "跟随", cwdAliasesMenu: "服务器主机名…", followHint: "跟随此终端的目录提示；手动打开目录会关闭跟随。", waitingCwd: "等待 Shell 目录提示",
    cwdAliases: "服务器主机名", cwdAliasesHint: "如果服务器自报的主机名与连接地址不同，在这里填写它的主机名（最多 8 个，英文逗号分隔），“跟随终端”才会接受它的目录提示。Shell 输出的名称不会自动获得信任。", cwdAliasNames: "主机名", cwdBindAliases: "保存名称",
    cwdAliasesInvalid: "请输入主机名或 IP 地址，以英文逗号分隔。去掉用户名、端口、路径和名称内部的空格后重新保存。", cwdAliasesLimit: "最多填写 8 个主机名，请减少后重新保存。",
    preferencesLoadFailed: "无法读取已保存的面板偏好，本次会话仍可调整布局。请检查 settings.json 格式和权限后重试。", preferencesSaveFailed: "面板偏好未能保存，本次会话仍保留当前布局。请检查应用数据目录的空间和权限后重试。", preferencesRetry: "重试",
    preferencesExitTitle: "面板偏好尚未保存完成", preferencesExitDescription: "文件操作尚未结束，窗口保持打开。你可以继续使用，或重试退出。", preferencesRetryExit: "重试退出",
    newFile: "新建文件", newDirectory: "新建目录", rename: "重命名", remove: "删除", permissions: "权限",
    edit: "编辑", editorOpening: "正在打开编辑器…", editorSaving: "正在保存远程文件…", editorConflict: "编辑期间远程文件发生了变化。", editorClose: "关闭编辑器", editorKeepRemote: "保留远端", editorReplace: "覆盖远端", editorSaveAs: "本地另存为…", editorNamePrompt: "新的远程文件名",
    shellIntegration: "Shell 集成", shellIntegrationTitle: "Shell 集成", shellIntegrationHint: "先预览受管理的配置区块，再修改当前选中的远程文件。", shell: "Shell", action: "操作", install: "安装", uninstall: "卸载",
    preview: "预览", applyIntegration: "应用集成", previewLoading: "正在读取远程配置…", previewSummary: (added, removed) => `将增加 ${added} 行，删除 ${removed} 行。`, noChanges: "无需修改。",
    name: "名称", size: "大小", type: "类型 / 权限", modified: "修改时间", next: "下一页", previous: "上一页", retry: "重新连接 SFTP",
    page: (number) => `第 ${number} 页`, omitted: (count) => `已略过 ${count} 个不支持的条目。其他文件仍可操作，请在终端中检查这些名称。`,
    loading: "正在读取远程目录…", empty: "目录为空，可新建文件或目录。", noVisible: "本页没有可见条目，可显示隐藏文件或打开下一页。",
    chooseName: "名称", chooseMode: "权限（八进制）", panelSize: "面板尺寸",
    createTitle: (directory) => directory ? "新建目录" : "新建文件", cancel: "取消", apply: "确认",
    renameTarget: (name) => `重命名为“${name}”`,
    permissionChange: (before, after) => `权限 ${before} → ${after}`,
    deleteScopeNote: "仅能删除空目录。",
    deleteTitle: (name, host) => `从 ${host} 删除 ${name}？`, deleteBody: "此操作无法撤销。",
    changeTitle: (name, host) => `修改 ${host} 上的 ${name}？`, sessionEnded: "SSH 会话已结束，重新连接后可继续浏览文件。",
    collapse: "折叠远程文件", expand: "显示远程文件", dock: "面板位置", right: "右侧", bottom: "下方", copied: "路径已复制。",
    production: "生产", moreActions: "更多文件操作", dockRight: "移到右侧", dockBottom: "移到下方", deleteKind: (kind) => `删除${kind}`,
    kinds: { file: "文件", directory: "目录", symlink: "链接", other: "特殊类型", unknown: "未知" },
    errors: {
      sftp_unavailable: "服务器拒绝了 SFTP，终端仍可使用。检查服务端配置后重新连接 SFTP。",
      permission_denied: "权限不足，请检查远程账户的权限后刷新。", path_invalid: "请输入绝对远程路径或有效的单个名称。",
      not_found: "远程路径不存在，请检查路径或刷新目录。",
      unsupported_file_type: "此名称或文件类型暂不支持，操作未执行。可在终端中检查。", target_conflict: "目标已变化或已存在，请刷新后重新确认。",
      resume_mismatch: "传输状态已不匹配，请检查源文件和目标。", local_io: "本地文件操作失败，请检查可用空间和权限。",
      remote_io: "远程操作失败或结果不确定，请刷新核实后再重试修改。", editor_unavailable: "配置的编辑器不可用。",
      editor_file_too_large: "文件超过 8 MiB 编辑上限，请使用文件传输。",
      editor_invalid_encoding: "请保存为有效 UTF-8 或带 BOM 的 UTF-16 后重试，本地副本仍保留。",
      editor_unsupported_content: "文件包含不支持的文本或控制字节，本地副本仍保留。",
      editor_conflict: "远端文件已变化，请先处理冲突再回传。",
      editor_stale_save: "本地文件仍在变化，请等待编辑器保存完成后重试。",
      editor_residual: "退出前请处理打开或恢复的编辑副本：先关闭外部编辑文档，再导出、保留文件供下次恢复或明确放弃。清理失败时会保留会话。",
      connection_lost: "SFTP 已断开，请重连 SSH 会话或重试 SFTP。", operation_canceled: "操作已取消。",
      scope_expired: "文件视图已过期，请重新连接 SFTP 获取当前目录。", resource_limit: "文件服务达到资源上限，请关闭不用的视图或稍候重试。",
    },
  },
};
