# AGENTS.md

本文件指导 AI 编码助手在 Capstan 仓库中工作。方案和决策记录见 [docs/PLAN.md](docs/PLAN.md)，有冲突时以 PLAN.md 为准；方案变更需先与维护者确认，再在 PLAN.md 的决策记录补一行。

## 项目概况与工作方式

Capstan 是维护者个人开发的 Windows 优先终端与 SSH 客户端，目标是替代 MobaXterm 日常使用。GitHub 开源，许可证 GPL-3.0-or-later。

- 技术栈：Tauri 2 + Rust（tokio、russh、portable-pty）+ React 19 / TypeScript / Vite / Tailwind 4 / xterm.js
- 当前状态和验收进度见 [docs/agent/board.md](docs/agent/board.md)
- 分工：AI 负责编写代码，维护者负责审核，并通过日常使用验收

工作方式：

- 先做到完成度足够日常使用，再边用边打磨。优先做影响日常使用的功能和 bug。
- 流程从轻：用自动化测试守住正确性、数据安全和安全，其余问题靠维护者实际使用发现。
- 拿不准范围时先给轻量方案，不要先搭验证体系。
- 保持改动聚焦，不做与当前任务无关的重构。

## 目录结构

```
crates/capstan-vault/     加密库
crates/capstan-ssh/       SSH 连接池、通道、SFTP 协议、端口转发、Agent
crates/capstan-pty/       本地 Shell 探测、ConPTY、WSL
crates/capstan-terminal/  共享终端流控、协议、会话任务与状态
crates/capstan-core/      应用核心：Vault 与连接管理、SFTP 服务、传输、远程编辑、Shell 集成、设置
src-tauri/                Tauri 命令、事件与窗口，只做粘合（crate 名 capstan-app）
ui/                       React 前端
vendor/                   带补丁的第三方依赖与原生 Windows 适配，改动说明见各目录的 CAPSTAN_PATCH.md
scripts/                  构建、打包、ConPTY 准备和测量脚本
docs/                     方案、格式与使用说明；docs/agent/ 为状态页和历史记录
```

## 常用命令

```powershell
pnpm -C ui install --frozen-lockfile
powershell -NoProfile -File scripts/Prepare-ConPty.ps1   # 准备捆绑的 ConPTY 运行时
pnpm tauri dev                                            # 开发模式运行
```

交付前检查：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --features tauri-runtime -- -D warnings
cargo test --workspace
cargo test -p capstan-app --features tauri-runtime
pnpm -C ui typecheck
pnpm -C ui test
```

按改动补跑：

- 改 `vendor/` 下的依赖：对该目录执行 `cargo fmt`、`cargo clippy`、`cargo test`（`--manifest-path vendor/<name>/Cargo.toml`）
- 改终端或 SFTP 面板布局：`pnpm -C ui test:layout`、`pnpm -C ui test:sftp-layout`
- 改终端安全策略（剪贴板、链接、粘贴）：`pnpm -C ui test:security`
- 改测量相关代码：`cargo test --workspace --all-features`
- 增删或升级依赖：`cargo deny check all`、`powershell -NoProfile -File scripts/Check-Licenses.ps1`

完整检查列表以 [.github/workflows/windows.yml](.github/workflows/windows.yml) 为准。

## 架构硬性规则

1. `crates/*` 不得依赖 `tauri`。业务逻辑写在 crate 里，`src-tauri` 只做参数转换和转发。
2. 终端字节流必须通过 Tauri Channel 以原始二进制传输，禁止用 `Vec<u8>` 走 JSON 序列化或 Base64。输出有界合并发送；必须实现按字节 ACK 的背压，ACK 在 xterm 解析完成后发送，积压统计覆盖 Rust 队列、IPC 在途和前端待处理字节。当前默认值为 16 ms 或 64 KB 合并、积压 > 1 MB 暂停读取、< 256 KB 恢复；数值可按实测调整，调整时在 PLAN 决策记录留痕。
3. 兼容的连接配置复用一条 SSH 连接（复用键见 PLAN §2.4），shell、SFTP、状态采集、隧道都是这条连接上的通道；同一连接最多 3 个 SFTP 子系统。不得为文件操作另建 TCP 连接，或通过隐藏 shell 解析 `sftp` 命令输出。
4. 只有可见窗格挂载 xterm.js 的 WebGL 渲染器，隐藏时释放。
5. 本地 PTY 优先使用捆绑的 ConPTY，加载失败时回退到系统版本。

## 安全与数据规则

- 前端不接触明文凭据，只传主机 ID 或凭据 ID；密码和私钥只在 Rust 侧解密，使用 `secrecy` / `zeroize` 管理。
- 不启用 Tauri 的通用 shell、fs 插件；只暴露自定义命令，权限按最小化配置，CSP 不放开远程内容。
- 文件正文只在 Rust 侧读写，不经过 WebView；本地文件只能通过原生选择器或拖放授予的受限 token 访问，前端不能提交任意本地路径。
- 远程和本地写入（上传、下载、编辑回传、Vault、Shell 集成）先写临时文件，验证后再替换；失败或取消时不得破坏原文件。
- 主机指纹变化必须阻断连接，不提供"静默接受"选项。
- 日志中不得出现密码、私钥、口令和终端内容。
- 不要提交任何密钥、证书或测试用私钥以外的凭据；测试密钥只能在测试运行时生成。

## 编码约定

- 代码、注释、标识符、提交信息使用英文；用户可见文字一律走 i18n（`zh-CN` 和 `en`），不在组件里硬编码。
- Rust：库 crate 用 `thiserror` 定义错误，应用层可用 `anyhow`；库代码不使用 `unwrap()` / `expect()`（测试除外）；异步统一用 tokio。
- TypeScript：开启 `strict`，不用 `any`；状态管理用 Zustand；样式用 Tailwind，组件基于 Radix UI。
- 新增依赖要固定版本，许可证必须与 GPL-3.0 兼容（MIT、Apache-2.0、BSD、ISC、Zlib、MPL-2.0 等），禁止 GPL-2.0-only。

## 测试

- 修复 bug 或新增功能时要附带测试，重点覆盖正确性、数据安全和安全边界。
- SSH / SFTP / 隧道的集成测试使用 russh 自带的服务端在进程内启动，不依赖 Docker。
- 真实 OpenSSH 兼容性测试连接 WSL 中的 sshd，通过环境变量 `CAPSTAN_TEST_SSHD`（如 `user@127.0.0.1:2222`）启用，未设置时跳过。
- 完成任务前运行上面的交付前检查，全部通过再交付；没跑或跑不了的项目如实说明，不要记成通过。
- 性能不是交付或发布门槛。只在维护者提出，或改动终端、传输的核心路径时测量：用现有脚本对比改动前后，把结论和关键数字写进交付说明。不冻结阈值，不做多轮测量矩阵。
- 功能完成后给维护者一份简短手测清单（十几项以内），维护者实际使用即为验收。

## 文档

- [docs/PLAN.md](docs/PLAN.md)：方案与决策记录。
- [docs/agent/board.md](docs/agent/board.md)：唯一的当前状态页，保持一屏以内，状态变化时更新。
- [docs/SFTP.md](docs/SFTP.md)、[docs/VAULT-FORMAT.md](docs/VAULT-FORMAT.md)、[docs/PORTABLE.md](docs/PORTABLE.md)：功能与格式说明，行为变化时同步更新。
- `docs/agent/fixtures/*.json` 和 `docs/agent/baselines/M2/performance-policy.json` 会被 Rust/TS 测试直接读取，属于现行契约：改动对应接口或资源上限时同步更新。policy 里的 WinSCP 比例和终端回归阈值只作参考目标。
- docs/agent/ 下其余文件（M0–M2 计划与报告、`board-history.md`、`baselines/` 下的其他内容）是历史记录，只作参考，不再追加或更新。
- 不再新建原始证据目录，不往 docs 复制源码、依赖或构建产物；需要留数据时，写一段结论加关键数字。部分测量和打包脚本默认把输出写进 `docs/agent/baselines/*/raw/`，运行时用 `-EvidenceDirectory target/<名称>` 改到 `target/` 下。
- 文档写短：结论先行，一句话说一件事。

## Git 与协作

- 提交信息遵循 Conventional Commits（如 `feat(ssh): add agent auth`）。
- 未经维护者要求不要提交或推送；不要直接推送到 `main`。
- `.gitattributes` 将文本文件统一为 LF。
- 开发环境为 Windows 11，Shell 为 Git Bash 和 PowerShell，注意路径分隔符和换行符。
