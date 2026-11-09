# Capstan

Capstan is a Windows-first terminal and SSH client built with Tauri 2, Rust,
React and xterm.js. It provides local PowerShell and WSL terminals, saved SSH
hosts, encrypted credential storage, host-key verification, independent shell
tabs over shared SSH connections and an SFTP side panel.

M0–M2 are complete for the verified Windows x64 scope. SFTP supports browsing,
resumable transfers, remote editing, shell integration and directory following.
M3 adds split panes, input broadcasting, snippets and SSH tunnels. These
features have passed automated checks and are awaiting hands-on acceptance.
See [the current status](docs/agent/board.md) and
[file workflows and recovery](docs/SFTP.md).

## Build on Windows

Use Windows 11 x64 with WebView2 Runtime, the Visual Studio C++ build tools and
Windows SDK, Rust 1.96.0, Node.js 22, pnpm 10.28.1, and CMake. NSIS and Python
3.12 are required for installer builds and their embedded payload verification.
AWS-LC uses the pinned dependency's prebuilt NASM objects
on supported Windows x64 builds; it still requires the C/C++ toolchain.

```powershell
pnpm -C ui install --frozen-lockfile
powershell -NoProfile -File scripts/Prepare-ConPty.ps1
pnpm tauri dev
```

```powershell
pnpm bundle:nsis
powershell -NoProfile -File scripts/Build-Portable.ps1 -OutputPath target/bundle/portable/capstan-local.zip
```

NSIS output is under `target/release/bundle/nsis`. The portable packaging script
refuses to overwrite an existing archive. Artifacts are unsigned; see
[portable and installed data locations](docs/PORTABLE.md).

Software updates are manual by default and require a configured trusted updater
public key. This checkout leaves that key empty, so updates are unavailable.
Portable builds only announce updates. See [updates and recovery](docs/UPDATES.md)
for signing, installation and key rotation.

## Build on macOS

Use macOS 14 or later with the Xcode Command Line Tools, Rust 1.96.0, Node.js 22
and pnpm 10.28.1. Node 25 and later need `NODE_OPTIONS=--no-webstorage` for
`pnpm -C ui test`.

```bash
pnpm -C ui install --frozen-lockfile
```

```bash
pnpm tauri dev
```

```bash
pnpm bundle:mac
```

Bundles are under `target/release/bundle` and are neither signed nor notarized, so
macOS asks for confirmation the first time one is opened. The dmg step scripts Finder for
its window layout; where that is not permitted, run `CI=true pnpm bundle:mac` to skip it. Data is stored in
`~/Library/Application Support/`, under the app identifier.

macOS provides local login shells, SSH (including the agent at `SSH_AUTH_SOCK`), SFTP
transfers, drag-in upload, tunnels and the encrypted vault, with Command-based shortcuts
and Option as Meta. Remote editing,
drag-out to Finder, resuming interrupted transfers after a restart, system
notifications, Touch ID and updates are Windows-only for now.

## Use and protect your data

Local terminals work while the Vault is locked. Creating or unlocking the Vault
enables saved hosts and new SSH connections. Passwords and private-key contents
are collected in native dialogs and remain in the Rust process. Verify the
server's fingerprint through a trusted channel before accepting its first key;
a changed key blocks connection.

The arrow beside the new-terminal button selects an installed PowerShell or
WSL distribution and an optional start directory. WSL directories use absolute
Linux paths; leave the field blank for the shell default.

Portable data lives beside the executable under `data/`; installed data lives
under `%APPDATA%\Capstan`. Keep that data when replacing the executable. Exported
`.cbak` backups use their own password. Recovery and format details are in
[VAULT-FORMAT.md](docs/VAULT-FORMAT.md). File-based private-key references and
Agent identities require their original local resources after migration.

## Development and verification

See [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md),
[CHANGELOG.md](CHANGELOG.md), and [the M1 compatibility matrix](docs/agent/M1-SSH-COMPAT.md).
The project is licensed under [GPL-3.0-or-later](LICENSE).

## 中文

Capstan 是 Windows 优先的本地终端与 SSH 客户端，目前提供 PowerShell/WSL 终端、本地
加密保存的主机和凭据、主机指纹验证、多标签、同一连接上的独立 Shell，以及 SFTP
侧栏。M0–M2 已在 Windows x64 范围内验收，SFTP 支持浏览、断点续传、远程编辑、Shell
集成与目录跟随。M3 新增分屏、输入广播、命令片段和 SSH 隧道，已通过自动化检查，等待
维护者手测。当前状态见 [项目看板](docs/agent/board.md)，操作见
[远程文件与恢复](docs/SFTP.md)。

构建前安装 Windows 11 x64、WebView2、Visual Studio C++ 构建工具与 Windows SDK、
Rust 1.96.0、Node.js 22、pnpm 10.28.1、CMake；生成安装包及内嵌文件校验还需要 NSIS 和 Python 3.12。开发与打包
命令见上方。产物尚未签名，安装/便携版数据位置见 [便携说明](docs/PORTABLE.md)。

软件更新默认手动检查；当前源码未配置生产更新公钥，功能不可用。便携版只提示新版本。
签名发布、安装失败恢复与密钥轮换见 [更新说明](docs/UPDATES.md)。

macOS 需要 14 或更高版本、Xcode Command Line Tools、Rust 1.96.0、Node.js 22、pnpm 10.28.1，
命令见上方的 Build on macOS。macOS 版提供本地登录 Shell、SSH、SFTP 传输、拖入上传、隧道和
加密库，快捷键以 ⌘ 为主，⌥ 作为 Meta。远程编辑、拖出到访达、重启后续传、系统通知、Touch ID
和软件更新暂时只有 Windows 版。

本地终端无需解锁加密库；保存的主机与新 SSH 连接需要解锁。首次连接需独立核验主机
指纹，密钥变化会阻断连接。更新便携程序时保留完整 `data/`；备份导出使用独立口令。
恢复细节见 [Vault 格式说明](docs/VAULT-FORMAT.md)。文件引用私钥和 Agent 身份在迁移后
仍需对应本机资源。项目许可证为 GPL-3.0-or-later。

新标签按钮旁的箭头可选择已安装的 PowerShell/WSL 和起始目录；WSL 使用 Linux
绝对路径，留空使用 Shell 默认目录。移动便携程序时同时保留 `runtime/` 和 `data/`。
