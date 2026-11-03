# Changelog / 更新记录

## Unreleased — M2 engineering preview

- Add SFTP browsing and reviewed file operations, bounded parallel file/folder
  transfers, native Explorer drop/export, restart recovery and verified resume.
- Add remote-editor save/conflict/recovery workflows, per-terminal OSC 7
  directory following and reversible Bash/Zsh configuration transactions.
- Preserve cleanup-only or inspection records after failed partial removal;
  reserve export capacity before staging and disable expired export controls.
- Reject upload deletion through a stale resumable checkpoint after failed
  persistence/restart; retain source-verified resume. Roll back owned export
  initialization failures with bounded cleanup.
- Pin Tokio to 1.53.3, crypto-bigint to 0.7.6 and ctutils to 0.4.5 for disclosed
  security fixes. Preserve audits and quantitative results by source identity.
- Close the scoped independent code review and prepare version-only 0.2.0
  engineering installer/portable/upgrade verification. Foreground performance,
  affected desktop acceptance and release-candidate sign-off remain pending.
  Current execution is recorded in [M2 follow-up](docs/agent/M2-FOLLOWUP.md).

### 中文

- 新增 SFTP 浏览、文件操作、并行文件/文件夹传输、Explorer 拖入/拖出及重启恢复和校验续传。
- 新增远程编辑、保存冲突与副本恢复、按终端跟随目录和 Bash/Zsh 配置安装/卸载事务。
- 修复清理失败丢失恢复记录、并发导出遗留暂存和授权到期后仍可拖拽的问题。
- 修复持久化失败并重启后旧续传记录恢复删除权限的分支；保留来源校验续传，并对导出初始化失败执行有界清理。
- 固定 Tokio 1.53.3、crypto-bigint 0.7.6 与 ctutils 0.4.5；审计和定量记录均保留各自源码身份。
- 完成范围内独立代码复核，并准备 0.2.0 隔离工程包的安装、升级和便携验证；前台性能、实际桌面及正式候选仍待签收，当前执行见本轮收尾。

## M1 candidate — accepted, unpublished

- Add encrypted Vault storage, native credential entry, saved hosts/groups,
  revisioned trust management, password/private-key/Agent authentication and
  independent terminal channels over compatible shared SSH connections.
- Add terminal search, clipboard/paste/link policies, lock options, independent
  encrypted backups, authenticated recovery selection and portable/NSIS builds.
- Fix startup state initialization, terminal height, multiple forwarded shells,
  host-trust confirmation deadlines and SSH startup error reporting.
- Replace the RSA private-operation backend with AWS-LC; validate imported
  components, retain SHA-2 authentication and remove the vulnerable `rsa`
  dependency and advisory exception.
- Separate production CSP from development-server permissions; expand real
  OpenSSH coverage to RSA OpenSSH/PKCS#1/PKCS#8 formats and RSA host keys.
- Complete M1 acceptance with maintainer device confirmation, verified bundled
  ConPTY/native-host companions, a native PowerShell/WSL directory launcher and
  fixed visible local/SSH performance evidence. Enable TCP_NODELAY for interactive
  SSH traffic. Verify the final unsigned x64 NSIS/portable candidate; no release
  publication is implied. See [M1-B.md](docs/agent/M1-B.md).

### 中文

- 新增加密库、原生凭据输入、主机/分组、指纹管理、多种认证、连接复用与独立终端。
- 新增搜索、剪贴板/粘贴/链接策略、锁定、独立加密备份、恢复与安装/便携打包。
- 修复初始化、终端高度、多 shell Agent 转发、指纹确认超时和 SSH 错误误报。
- RSA 私钥运算改用 AWS-LC，移除原漏洞依赖与审计例外；生产 CSP 单独收紧。
- M0/M1 实机范围按维护者确认通过，补齐已核验 ConPTY、普通 WSL/目录入口、
  固定可见本地/SSH 性能与最终安装/便携候选；启用 TCP_NODELAY，M1 已收尾。
  未实际发布，原始证据和限制见 M1-B。
