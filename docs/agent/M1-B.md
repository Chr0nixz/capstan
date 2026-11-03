# M1 实施记录

## 最终收尾（2026-10-17）

**结论：M0 前置项、M1 A/B 门和整体阶段均为 `accepted`；已具备发布 v0.1 候选并进入 M2 的条件。** 本次补齐实际工程与交付缺口；没有发布、创建 tag、提交或推送。`docs/PLAN.md` 未修改。下方旧的 `blocked` 记录属于相应检查时点，以本节和 [board](board.md) 为当前结论。

维护者此前报告 M1 实机测试全部通过；本次进一步确认 Snap/Mica、CJK/emoji、DPI/多显示器和 WebGL 恢复“这些也已通过”，[原话与范围](baselines/M1/raw/2026-10-17-maintainer-m0/README.md) 已归档。按维护者报告接受这些手测，不要求重复，不补造逐项截图、版本或原始性能数字。

### 修改内容

- 锁定官方 ConPTY `1.25.260930003`：固定 NuGet SHA-256，逐一核验三个二进制的微软签名、PE 架构与文件版本，附 MIT 许可证和来源清单。x64 DLL 与 `x64/arm64` 两个宿主 companion 按原包目录布局进入 NSIS/便携包；ARM64 companion 不代表 ARM64 应用已经验证。缺失/篡改 DLL、companion、manifest 或许可证时打包拒绝。
- 新增普通 UI 的 Shell/WSL 与起始目录入口，沿用现有新标签按钮；本地终端在锁库时仍可用。Rust 返回脱敏 profile，校验 WSL 绝对 Linux 目录，以独立参数检查路径；前端显示明确的无效目录错误。默认 PowerShell、标签生命周期和原始二进制流控保持同一实现。
- 扩展显式启用的测量工具，使用一次性 Vault、密码和 Ed25519 身份、预置核验的服务端公钥，以及真实 WSL sshd。三轮 20 标签包括同连接多 shell、不同身份隔离、本地/SSH 混合及慢 ACK 邻接 shell；只归档计数、校验与环境数据。公钥文件注释归一化，8+8 shell 避免超出 OpenSSH 默认会话上限。
- 修复 SSH 交互延迟：启用 `TCP_NODELAY`，记录实际协商算法。失败的性能样本和冻结阈值保留，最后一轮没有放宽门槛。
- 构建流程自动准备运行时，修复 Tauri 嵌套 PowerShell 的系统模块解析；安装版同时分发 GPL、第三方声明和数据目录说明。新增包内容/生产测量入口禁用核验，更新 CI、构建与复现说明。

### 测试结果

| 检查 | 结果与证据 |
| --- | --- |
| fmt、补丁 ssh-key fmt | 通过 |
| Clippy 默认 / 全部特性 | 均零告警通过；全部特性包含 Tauri runtime 和测量模块 |
| Rust workspace / Tauri runtime / 全部特性 | 120 / 14 / 134 项通过；[工程日志与退出码](baselines/M1/raw/2026-10-17-final-checks/checks.json) |
| UI 类型 / Vitest | 通过，84 项测试 |
| 真实浏览器布局 / 安全 | 28 / 10 项通过；布局含桌面与最小窗口的 Shell 选择入口；[布局](baselines/M1/raw/2026-10-17T07-40-42-143Z-terminal-layout/report.json)、[安全](baselines/M1/raw/2026-10-17T07-56-32-957Z-terminal-security/report.json) |
| 官方捆绑 ConPTY + PowerShell/WSL | 明确配置 Ubuntu 与已核验 DLL，3 项实际启动/目录/resize/退出测试通过，无跳过；[日志](baselines/M1/raw/2026-10-17-final-checks/bundled-wsl-powershell.txt) |
| 运行时负例 | 完整包、篡改 DLL、缺 companion、篡改 manifest、缺许可共 5 场景通过；[报告](baselines/M1/raw/2026-10-17-runtime/validation-tests.json) |
| 真实 OpenSSH | Ed25519 / RSA 服务器主机密钥各 12 场景通过并清理夹具；[Ed25519](baselines/M1/raw/2026-10-17-final-sshd/report.json)、[RSA](baselines/M1/raw/2026-10-17-final-rsa-host-sshd/report.json) |
| 依赖审计 / 许可 | cargo-deny 四类检查通过，advisory ignore 为空；388 个 Cargo 与 231 个前端包许可核验通过 |
| 可见性能 | 本地参考 62 会话/186 探针；混合 63 会话/192 探针，三轮 4 本地+8 密码 SSH+8 Ed25519 SSH，28 项比较检查全部通过；[方法、数据和阈值](baselines/M1/PERFORMANCE.md) |
| 安装 / 升级 / 卸载 | 普通用户 0.1.0→测试 0.1.1、加密数据保留、安装目录运行时核验和卸载移除运行时均通过；[报告](baselines/M1/raw/2026-10-17-final-install/report.json) |
| 便携 / 包内容 | 启动、移动、受阻数据路径退出 3 项通过；10 个文件、运行时/许可/哈希与生产测量入口禁用均通过；[桌面](baselines/M1/raw/2026-10-17-final-desktop/report.json)、[包核验](baselines/M1/raw/2026-10-17-final-package/report.json) |

默认 Rust 测试中未配置的外部入口仍明确跳过；真实服务器及捆绑 WSL/ConPTY 结论来自另行配置的实跑。新 24 个 OpenSSH 场景不冒充新的 Windows Agent/Pageant 测试；两者沿用先前自动化与已接受的维护者反馈。独立 KDF 参数测量仍见既有 [KDF 报告](baselines/M1/raw/2026-10-13-202719-kdf/report.json)，本次未改变 Vault KDF。

性能复测：隔离本地 28.279 MiB/s；混合输入中位数 77.9 ms、p95 129.3 ms；慢 ACK 正常邻接 shell p95 34.3 ms。每轮本地与 SSH 各生成 100 MiB，原始二进制、顺序/校验、解析/ACK 一致；最大可见 renderer 为 1，关闭后 renderer/session 为 0，SSH 池断开后为 0。峰值进程树工作集 925,589,504 字节、句柄 5,417。完整方法、失败轮次、设备、版本和冻结数字见上方性能报告；不将测量工具的解析回调延迟等同于物理键盘或 GPU 呈现延迟。

### 固定候选与替换

| 产物 | 字节数 | SHA-256 |
| --- | ---: | --- |
| `target/bundle/m1-final-20261017/Capstan.exe` | 18,749,440 | `52D8E3F97F48C06D02EAFCDBEF1BC9F1E5442B7296865608A79A4466ED7B22C2` |
| `target/bundle/m1-final-20261017/Capstan_0.1.0_x64-setup.exe` | 5,264,270 | `2299B2151297DEDCEE16BAB8A054E1AA7933A790822FF846099A555997A1224A` |
| `target/bundle/portable/capstan-0.1.0-win-x64-m1-final-20261017.zip` | 7,579,001 | `E54B2549DA50D98863E7854F433D263C8CBA40F719F6DB8DDC7293D24BB0D47E` |

便携包解压后文件合计 21,154,209 字节；当前账户安装后文件合计 21,233,702 字节，均不含系统 WebView2。`0.1.1` 只是升级夹具，不是发布版本。完整清单、源文件哈希、检查和原始证据见 [最终 manifest](baselines/M1/raw/2026-10-17-final/manifest.json)。

沿用“直接替换”的授权，已更新 `target/m1-manual-app-c9982bd2779b` 的程序、完整 runtime 和随包说明，备份旧程序为 `Capstan.before-m1-final-20261017.exe.bak` 并重开。Vault、三份加密备份、锁文件共 5 个文件及便携标记哈希未变；[替换报告](baselines/M1/raw/2026-10-17-final-replacement/report.json) 只证明替换与启动，没有自动输入凭据或扩大手测结论。

### 已知风险与未决问题

- 本次支持范围为 Windows x64；未签名，未宣称 ARM64 应用、干净 OS/新用户 VM 或远端 CI 的新增自动化证据。安装/便携的 WebView2 前置条件见 [说明](../PORTABLE.md)。
- 性能是本机、固定视口和 WSL loopback 的合成负载；本地输入 p95 103.4 ms 接近 104.6 ms 冻结上限，后续开发需保留回归。TCP connect 样本不是 ICMP RTT；不将它或解析探针冒充 WAN/物理输入/GPU 测量。
- 维护者手测为合并反馈，原始逐项记录未保存；本次补充确认没有生成不存在的旧记录。RSA 补丁的测试与依赖审计不等同于新增独立密码学审计。Vite 主 chunk 体积提醒仍存在。
- **无未关闭的 M1 阻断项或必须重做的维护者手测。** SFTP 等按既有计划进入 M2；实际发布、tag、提交和推送仍待维护者另行指示。本次未修改验收标准或 `PLAN.md`，其 SHA-256 为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。

---

以下为历次实施和验收记录；其中待测/阻断状态按当时日期保留。

## 完成度再次复核（2026-10-17）

**结论：M1 功能与维护者实机验收已通过，完整阶段仍为 `blocked`。** 按 `M1.md` §2.2、§7.2、§9.3 逐项核对，尚有以下真实交付缺口，不是重新要求维护者完成已报告通过的手测。

| 剩余项 | 本次直接核对结果 | 归属 |
| --- | --- | --- |
| 实际 ConPTY 捆绑 | 当前便携包只有 exe、标记和三份说明/许可文件，共 5 条目，没有 `runtime/<arch>/conpty.dll`；`Build-Portable.ps1` 没有复制运行时，Tauri bundle 未配置运行时资源。现有优先加载/系统回退代码不等于已交付捆绑包。 | M0 / M1-00 工程与包来源核验 |
| 普通 UI 的原生 WSL 启动 | `ui/src/workspace.ts` 的本地标签固定为 `{ kind: "local", options: {} }`，`addTerminal()` 不接收启动配置；Rust 侧已有 `wsl:<distribution>` 解析，但普通产品入口不能选择发行版/目录。 | M0 / M1-00 入口实现与定向测试 |
| 固定可见性能交付 | 现有 2026-10-16 全链路报告 `window_visible=false`，场景为 bulk、delayed_ack、三轮 tab_lifecycle，62 个本地 PTY；不是可见 SSH 与本地混合标签的比较原始数据。维护者已明确新记录“未保存”，无新增原始数据可补齐 §7.2 或据此冻结回归阈值。 | M1-13 / B8 性能证据 |

本次还核验了 247 个源码/配置文件的聚合 SHA-256 `0612E4D83D1DE9D9D01742E24EB169A7EE43A353CE8130242EF25AF4AC748F2D`，与已通过工程检查的 2026-10-16 快照一致；Cargo/前端锁文件及三个固定候选产物哈希均一致。读取的两份真实 OpenSSH、安装及便携 smoke 报告均为通过。既有 115/14/77 项测试结果继续作为对应版本证据，本次未重跑程序测试，不新增通过次数或实机声明。

复核结构化结果见 [report.json](baselines/M1/raw/2026-10-17-completion-review/report.json)。本次仅更新复核文档，保留原始测试报告；`docs/PLAN.md` 未修改。补齐上述工程/包与性能交付前，不能给出“完整 M1 已完成、可发布 v0.1 并进入 M2”的结论。

## 剩余实机验收全部通过的维护者反馈（2026-10-16）

维护者在自动化收尾和程序替换后反馈：“实机测试全部通过”；询问原始报告、三轮性能数据和已核验 ConPTY 包的保存位置后，维护者回复：“未保存”。按此前列明的 M1 剩余实机清单，登记原生安全交互、日常工作流、恢复/跨机迁移、安装/便携与可见压力测试为**维护者报告通过**，不再列为待执行或要求重复测试。[反馈与证据边界](baselines/M1/raw/2026-10-16-maintainer-full-manual/README.md) 已归档。

- A1–A6 既有自动化结果与本次 A7 真实闭环反馈合并后，A 门功能验收记录为 `accepted`；B1–B7 的剩余实机功能范围也记录为维护者报告通过。此前 P1/P2/P3 的结果继续有效。
- 本次是合并反馈，没有逐项原始记录、截图、新的环境/Agent 版本、独立包哈希确认或数值采样。没有生成、重跑或改写自动化结果；现有 115/14/77 项工程结果及已测候选身份保持不变。
- **完整 M1 仍为 `blocked`，原因已从“剩余实机待测”改为交付证据与 M0 前置工程缺口。** `M1.md` §7.2 要求的固定可见本地/SSH 100 MiB、三轮混合 20 标签、环境与比较原始数据未保存，不能由“通过”补出数值，也不能用隐藏窗口报告冒充。M0/M1-00 的有效捆绑 ConPTY 包/来源核验、原生 WSL 入口等既有工程交付缺口也不因手测反馈自动实现。
- 未新增未通过的功能测试，不要求维护者重测；也未改变验收标准、接受未声明架构或自动发布。完整阶段/发布签收需按既有契约补齐上述交付物。新 RSA 后端的代码审核不冒充新增独立密码学审计。

本次只更新验收记录、状态和交付说明，未修改代码、旧原始报告或 `docs/PLAN.md`，未提交或推送。PLAN SHA-256 保持 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。下方各节是对应时点的历史状态，以本节为最新验收结论。

## 自动化与交付收尾（2026-10-16）

按维护者“那你来收下尾”的要求，完成下列代码、工程、兼容性与安装验证。**P1/P2/P3 重点手测已验收，不需要重做；RSA 依赖发布阻断已移除。代码与本机可自动执行的交付检查已收尾，完整 M1 仍待剩余实机范围和最终审核，不能据此直接发布 v0.1。** 本节是当前状态，后续各日期段落保留历史结论。

### 修改内容

- 用本地 `vendor/ssh-key` 补丁将 RSA 生成、私钥一致性校验、SHA-256/SHA-512 签名和公钥验证接到固定版本 `aws-lc-rs 1.18.1`，并调整 `vendor/russh` 的 PKCS#1/PKCS#8 导入导出。russh 传输后端仍选 `ring`，AWS-LC 只用于这条 RSA 密钥路径。OpenSSH 缺失的 CRT 指数只在本地导入时以公开模数宽度、常量时间余数运算补齐，私密临时量使用 `Zeroizing`；解码时缓存已校验后端，签名不重新推导组件。拒绝不一致私钥、弱私钥和 SHA-1 新签名，保留旧 SHA-1 公钥验证以兼容服务器。来源、许可证与边界见 [补丁说明](../../vendor/ssh-key/CAPSTAN_PATCH.md)、[russh 补丁](../../vendor/russh/CAPSTAN_PATCH.md) 和 [第三方声明](../../THIRD_PARTY_NOTICES.md)。
- 从实际依赖图和 `Cargo.lock` 删除 RustCrypto `rsa`，移除 `deny.toml` 的 RUSTSEC-2023-0071 例外。该漏洞的[上游修复状态](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)仍未解决；本次通过替换实现移除相关依赖，不声称上游已修复，也不将兼容性测试当作密码学审计。
- 发布 CSP 仅允许自身与 Tauri IPC；Vite 的 HTTP/WebSocket 端口只保留在 `devCsp`。Rust 与 UI 测试同时约束两种配置，未加入通用 shell/fs 权限。
- 扩展一次性 WSL OpenSSH 夹具，运行时生成五种 RSA 私钥格式，支持独立 RSA 服务器主机密钥；保留密码、Ed25519、信任持久化与换钥阻断用例。
- 新增真实浏览器/xterm 安全集成入口 `pnpm -C ui test:security`，覆盖 OSC52 禁读/写入同意/分片/坏编码、多行粘贴拒绝/同意及 bracketed paste、非 HTTP 链接确认和监听器释放。使用隔离剪贴板适配器，不读写维护者剪贴板，不开放产品测试入口。
- 新增受保护的 [安装验收脚本](../../scripts/Test-M1Install.ps1) 和运行时生成加密数据的 [测试夹具](../../crates/capstan-core/examples/create_install_fixture.rs)。脚本拒绝管理员、已有安装、已有 roaming 数据或运行中的 Capstan；只在无既有数据的当前普通账户执行测试，不接触原便携库。
- 补齐根目录中英双语 [README](../../README.md)、[CONTRIBUTING](../../CONTRIBUTING.md)、[SECURITY](../../SECURITY.md)、[CHANGELOG](../../CHANGELOG.md)，更新 Windows CI 的 CMake、补丁格式与终端安全检查。远端 CI 尚未实际运行。

### 实际测试结果

| 检查 | 结果与证据 |
| --- | --- |
| Rust workspace / Tauri runtime | 115 / 14 项通过；默认两条外部入口按未配置环境跳过，实际服务器验证见下两行。[workspace 日志](baselines/M1/raw/2026-10-16-closeout/workspace-tests.txt)、[runtime 日志](baselines/M1/raw/2026-10-16-closeout/rust-checks.txt) |
| 真实 OpenSSH，Ed25519 主机密钥 | 12 个已配置场景通过，无跳过：[report.json](baselines/M1/raw/2026-10-16-closeout-sshd/report.json) |
| 真实 OpenSSH，RSA 主机密钥 | 同样 12 个场景通过，无跳过：[report.json](baselines/M1/raw/2026-10-16-closeout-rsa-host-sshd/report.json) |
| 格式与 Clippy | workspace 和 ssh-key 补丁格式、默认及 Tauri runtime 两组 all-targets Clippy 均通过；新安装夹具也已编译运行并复验 |
| 前端 | 类型检查通过；12 文件 / 77 项单元测试通过；安全夹具单独严格类型检查通过；生产构建通过 |
| 真实浏览器安全策略 | 10/10 通过；完整 `test:security` 命令连同严格类型检查再次通过：[report.json](baselines/M1/raw/2026-10-16T13-17-15-155Z-terminal-security/report.json)；隔离剪贴板适配器不代替原生 WebView2 剪贴板操作验收 |
| 真实浏览器布局 | 100%/125% 两组共 24/24 通过：[report.json](baselines/M1/raw/2026-10-16T13-00-02-153Z-terminal-layout/report.json) |
| 依赖审计与许可 | `cargo deny check all` 的 advisories/bans/licenses/sources 均通过，advisory ignore 为空；SPDX 检查 388 个 Cargo / 231 个前端包通过。[审计日志](baselines/M1/raw/2026-10-16-closeout/cargo-deny.txt) |
| 普通用户安装→升级→卸载 | 实际 0.1.0→0.1.1，两个版本均创建并正常关闭主窗口；Vault、两份加密备份及锁文件共 4 个文件哈希全程一致，卸载保留数据，测试框架核验后清理自有 roaming 测试文件。[report.json](baselines/M1/raw/2026-10-16-closeout-install/report.json)；旧字段 `encrypted_files` 包含锁文件，脚本后续已改名 `preserved_files` |
| 原生便携包 smoke | 初次启动、移动后启动、数据路径被文件阻塞时失败关闭，3/3 通过。[report.json](baselines/M1/raw/2026-10-16-closeout-desktop/report.json) |
| 真实 ConPTY→WebView2→xterm 回归 | debug/隐藏窗口五场景通过：100 MiB、延迟 ACK、三轮各 20 会话，共 62 PTY、186 个输入探针；完整性/二进制/ACK/背压/EOF 与会话及 renderer 回收均经验证器通过。[原始报告](baselines/M1/raw/2026-10-16-closeout-terminal-pipeline/terminal-report.json)、[环境](baselines/M1/raw/2026-10-16-closeout-terminal-pipeline/environment.json)、[进程采样](baselines/M1/raw/2026-10-16-closeout-terminal-pipeline/process-samples.json)。不是可见性能基线或真实 SSH 混合标签验收 |
| 工具 | M0 报告契约、四个拒绝分支、两个恶意 PE 分支及 PowerShell 语法检查通过；Node 驱动语法通过 |

两组服务器均为 `OpenSSH_9.6p1 Ubuntu-3ubuntu13.20` / OpenSSL 3.0.13，夹具服务、账户与临时秘密已清理。本次没有再次操作 Windows OpenSSH Agent 或 Pageant；其维护者 P2 与此前 Pageant 实测仍保留各自证据。安装升级用 `target/m1-closeout-upgrade.json` 临时覆盖包版本，仅作升级夹具，不修改源码版本或发布 0.1.1。

### 当前候选包

| 产物 | 字节数 | SHA-256 |
| --- | ---: | --- |
| [0.1.0 便携 zip](../../target/bundle/portable/capstan-0.1.0-win-x64-m1-closeout-20261016.zip) | 6,417,743 | `785D1EA6F18F27880C90652F3942D98085E9926C059CC6AC095F547EBD87E2E6` |
| [0.1.0 NSIS](../../target/bundle/m1-closeout-20261016/Capstan_0.1.0_x64-setup.exe) | 4,411,306 | `C91F0E70F084BABF4A8AA00745CBA525F58D08657F744A2403F0C1EEF579CB55` |
| [0.1.0 exe](../../target/bundle/m1-closeout-20261016/Capstan.exe) | 18,635,776 | `84DE0E5BDC5D8B4CFA48F4F64185F1A452C512B540150DB85C2210D81962C17F` |

本轮包为未签名 Windows x64 候选。不可将仅供升级验收的 `Capstan_0.1.1_x64-setup.exe` 当作交付版本。目录 `target/bundle/m1-closeout-20261016` 固定保留本次已测产物，避免后续构建覆盖候选身份。

归档五个条目及内含 exe 哈希已核对；结构化证据与源码/锁文件标识见 [manifest.json](baselines/M1/raw/2026-10-16-closeout/manifest.json)。升级夹具结束后已重新构建默认 0.1.0，默认 release 目录不再是 0.1.1。重编译产物另记于 manifest，不覆盖上表已验收的固定包；当前未承诺不同重编译的二进制逐字节一致。

已沿用“直接替换”的授权，备份 `target/m1-manual-app-c9982bd2779b/Capstan.exe` 为同目录 `Capstan.before-m1-closeout-20261016.exe.bak`，只复制上表的新 exe。替换前后 Vault、三份加密备份与锁文件共 5 个文件及便携标记哈希一致；旧 exe 备份 hash 为 `A4E70454432E39C54B7AEFE7AC38F2E1FEE99EE7751F091F0D938C828BD53DD3`。重新打开原目录，5 秒后程序仍运行并创建 `Capstan` 主窗口（PID 18764）。[替换记录](baselines/M1/raw/2026-10-16-closeout-replacement/report.json) 与 [替换前快照](baselines/M1/raw/2026-10-16-closeout-replacement/before.json) 已保存；启动 smoke 不扩展已有 P1/P2/P3 维护者验收范围，无需重建 Vault 或测试账户。

### 已知风险和未决问题

1. RSA 后端替换已关闭旧依赖阻断，但新增供应链/FFI 与本地适配补丁仍需要正常代码审核；当前测试不是独立密码学审计。旧 SHA-1 签名、1024-bit 私钥及多素数私钥不受支持，不能承诺所有历史 RSA 文件兼容。
2. 原生 WebView2 的 OSC52、系统剪贴板、多行粘贴/链接确认，以及剩余主机/分组、搜索/主题、原生取消/失败与损坏恢复 UI，仍按先前暂缓清单保留未验收。新真实浏览器测试缩小了代码不确定性，未替维护者点击原生交互。
3. `.cbak` 跨机迁移仍缺另一 Windows 环境的实际导出/导入/冲突与文件引用失效验收；同机单元测试不替代跨机。当前普通用户的首次安装/两版升级/卸载已补齐，但不是干净 OS/新用户 VM；真实只读文件系统、其他架构仍未验证。
4. 固定条件的可见 M0/M1 性能比较、真实本地/SSH 混合 20 标签三轮、有效捆绑 ConPTY 包及 M0 其余设备范围仍未签收。不调整 PLAN 或事后冻结阈值，不将功能测试速度当性能达标。
5. 生产 JS 684.50 kB、gzip 193.20 kB，仍有单 chunk 大于 500 kB 警告；Windows x64 产物未签名，ARM64 和远端 CI 未运行，不宣称发布覆盖。

旧待删除目录 `target/m1-sshd-56dbde153625` 本轮检查已不存在，无需维护者继续处理那条旧指示。现有 C998 手测目录和一次性 sshd 材料保留，不读取或自动删除。Windows `ssh-agent` 当前观测为 Stopped/Disabled，本次没有修改服务状态。未提交、推送、打 tag 或发布；`docs/PLAN.md` 未修改，SHA-256 为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。

## 维护者重点手测全部通过（2026-10-16）

维护者在本轮修复和程序替换后反馈：“测试全部通过”。按当前 [重点手测指南](M1-priority-manual.md) 的必测范围，记录 P1、P2、P3 均通过；此前 P1 第 4 步“信任并保存”后失败的问题已由维护者复测关闭。

| 本轮范围 | 验收结果 |
| --- | --- |
| P1 加密库与密码 SSH | 维护者报告通过：原生录入、拒绝/保存信任、终端交互、错误解锁、重启持久化和两种显式锁定策略。 |
| P2 Windows OpenSSH Agent | 维护者报告通过：身份选择/认证、转发关闭→开启→再次关闭、远端签名、双 shell、身份丢失与恢复。 |
| P3 主机换钥 | 维护者报告通过：新旧指纹、重复阻断、独立核验后显式移除/重新信任与重启恢复。 |

反馈归档见 [维护者验收记录](baselines/M1/raw/2026-10-16-maintainer-priority/README.md)。该记录引用维护者原话，不将它包装成新增自动化执行、截图或环境采样；本轮提供的程序版本与 hash 见下方替换记录。之前各节的“P1/P2/P3 待手测”是当时状态，以本节反馈为准，无需维护者重复本轮重点。

完整契约中暂缓的跨机迁移、安装/升级/卸载、固定性能基线、其余桌面安全交互和最终审核继续保留原状态；A7 的远端剪贴板/粘贴策略不在本轮必测步骤内。可选的私钥 GUI、Agent 服务重启/拒签/取消，以及清理/服务还原未单独回报，不据此补填结果。既有安全发布阻断仍待处理，M1 总体尚不具备完整发布结论。

本轮只更新验收文档，没有修改程序或重跑既有工程测试。`docs/PLAN.md` 未修改，SHA-256 仍为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。

## P1 信任确认超时与错误提示修复（2026-10-16）

维护者在 P1 第 4 步确认“信任并保存”后看到“无法启动本地 Shell”，并确认用户名和本次测试账户密码来源一致。本轮保留现有手测 Vault 与专用 sshd，没有读取维护者 Vault 的明文凭据。

### 修改内容与复现结论

- `capstan-ssh` 的 15 秒 `client::connect` 超时覆盖了主机密钥确认回调，但 `TrustBroker` 为人工核验设定的是 120 秒。新增测试在收到指纹提示后推进 20 秒，旧实现已结束连接，测试明确失败；修复后该测试通过。现在核验指纹及保存信任时暂停网络预算，完成后继续使用剩余网络时间；人工确认仍受 120 秒限制，没有取消网络超时。
- 为每次建连创建子取消令牌与 drop guard。建连失败或调用方丢弃启动任务后，独立运行的 russh handler 也会取消并移除待确认请求，迟到的“信任”响应不能写入孤立尝试的信任记录。
- `capstan-terminal::TerminalError` 新增只承载固定公开错误标识的 `BackendCode`；应用层用它保留 SSH/Core 的安全错误码。动态 `Backend(String)` 仍只返回 `backend`，不将内部异常、路径或凭据传给前端。原始二进制 Channel、帧格式、ACK 和背压没有改动。
- 前端按错误码分别显示认证失败、超时、终端请求被拒、私钥/引用文件失败、Agent/身份不可用、锁库或取消。未知 SSH 启动错误也使用 SSH 连接提示，拒绝信任显示取消，不再回落到本地 Shell 文案。新增文案保持中英双语，认证失败说明适用于密码、私钥与 Agent。
- 新增 `crates/capstan-core/tests/ssh_workflow.rs`，从独立临时加密库的保存凭据、保存主机、拒绝/接受信任一直覆盖到认证、凭据更新和重新打开 Vault。测试不读取维护者的 Vault，不修改 `known_hosts`，密钥和测试口令在运行时生成。

### 测试结果

| 检查 | 实际结果 |
| --- | --- |
| 旧实现延迟确认复现 | `confirming_host_trust_after_network_timeout_still_connects` 失败：20 秒核验时间被 15 秒网络超时消耗；修复后通过 |
| SSH 进程内回归 | 23 项通过；含延迟确认、120 秒确认过期、丢弃启动任务后的清理、认证前零请求、网络预算恢复与既有通道/Agent/池测试 |
| Core 完整工作流 | 独立 Vault 拒绝→接受信任、错误密码拒绝、更新凭据与重新打开 Vault 后成功连接，通过 |
| 当前专用 WSL OpenSSH | `cm1c9982bd2779b@127.0.0.1:22292`；直接 SSH 库的认证/双 shell/UTF-8/resize 通过；新 Core 工作流实际等待 20 秒后确认并打开 shell、重新打开临时 Vault 后再次连接通过，2 项共 21.60 秒 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test --workspace --offline` | 112 项、0 失败；两项外部连接入口默认按环境配置跳过，真实服务器结果以上一行的定向运行计 |
| `cargo test -p capstan-app --features tauri-runtime --offline` | 13 项通过；含错误经过异步终端服务和命令适配器后的保留、动态错误详情脱敏 |
| 默认与 Tauri runtime 两组 Clippy | 均通过，`-D warnings` |
| `pnpm -C ui typecheck` / `test` / `build` | 通过；12 文件 / 77 项；新增 19 项启动错误分类/安全回退回归；JS 684.50 kB、gzip 193.20 kB，保留大于 500 kB 的 chunk 警告 |

服务端为 `OpenSSH_9.6p1 Ubuntu-3ubuntu13.19` / OpenSSL 3.0.13；定向工作流固定核验本次专用服务的 `SHA256:es8vN+gGk5pst3N5B9eQnEnGkCZHpmFwdqPHUlvf30Y`，通过 `CAPSTAN_TEST_SSH_HOST_FINGERPRINT` 输入。测试只在 Rust 内读取本次一次性密码文件，不打印密码、主密码、私钥或远端输出；该临时 Vault 与维护者实际手测 Vault 分离。

### 当前包与直接替换

已构建新的未签名 x64 [便携包](../../target/bundle/portable/capstan-0.1.0-win-x64-m1-ssh-trust-timeout-20261016.zip)，5,838,023 字节，SHA-256 `29FAE46E2634C6A758D7A3B2D65E0E78888AB959DA20B9A7DBAF70EE75D7C675`。五个归档条目完整，归档 exe 与 release exe 的 SHA-256 均为 `A4E70454432E39C54B7AEFE7AC38F2E1FEE99EE7751F091F0D938C828BD53DD3`，17,640,448 字节。NSIS 为 `target/release/bundle/nsis/Capstan_0.1.0_x64-setup.exe`，4,063,172 字节，SHA-256 `0BDC69BCBD3FE6DE63798453D7FE5172E0E06D21DE1C0623B0D51FB6FD007D32`。本包保留此前终端高度修复；本节包与 [重点手测指南](M1-priority-manual.md) 为当前入口，下文包是历史归档。

沿用维护者“直接替换”的授权，已识别这次实际运行目录 `target/m1-manual-app-c9982bd2779b`，备份旧 exe 为同目录 `Capstan.before-ssh-trust-fix-20261016.exe.bak`，以正常关闭请求退出 PID 65924，只复制新 `Capstan.exe`。替换前后 `data` 中的 Vault、三份加密备份和锁文件共 5 个文件的 SHA-256 均一致，便携标记也未改变。随后重开原目录的新程序，PID 58608，5 秒后仍运行并创建 `Capstan` 主窗口；这只证明替换与初始启动，不记为 P1/A7 交互验收。没有重建或清除测试 sshd、测试账户和凭据。

结构化结果与替换前数据快照见 [report.json](baselines/M1/raw/2026-10-16-ssh-trust-timeout/report.json) 和 [replacement-before.json](baselines/M1/raw/2026-10-16-ssh-trust-timeout/replacement-before.json)。旧 exe 备份与 `target/m1-replacement-20261016/Capstan.exe` 均保留，方便回退；没有执行递归删除。

### 已知风险与未决问题

已经复现并修复“核验指纹超过 15 秒”这一失败机制及误报链路，但旧桌面尝试没有保留下来的具体错误码，因此不能据此断言历史失败只有这一种原因。原生密码录入和维护者实际 Vault 中的这次 P1 连接仍需用新程序复试；后端测试不代替该验收。无需重建测试账户、删除信任或重建 Vault：关闭原失败标签后重新连接，已保存的同一指纹可能直接进入 shell。若再次失败，记录新的具体提示即可，不发送秘密或终端正文。

M1 的完整 A7、Windows OpenSSH Agent、安装/性能及既有安全发布阻断不因本次定向修复自动通过。没有调整 Vault 格式、依赖版本或 PLAN；`docs/PLAN.md` SHA-256 仍为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。

## 维护者手测目录直接替换（2026-10-14）

按维护者“直接帮我替换”的指示，定位到唯一的 `target/m1-manual-app-70d4c413e820` 便携手测目录；旧 exe SHA-256 为 `41685014ADE540F3F3FCE4A09E08A3CBD7FF7EFCE6586F84D3B620B255525432`，操作前没有运行中的 Capstan。已核对新 zip 与解压 exe 的固定 hash，备份旧 exe 到同目录 `Capstan.before-layout-fix-20261014.exe.bak`，只替换 `Capstan.exe`，替换后 SHA-256 为 `93EF56D1744D98EB777294EF64DF5AD6EA0D5F7FF69C8A220D3BA5C28F3C081B`。数据文件快照和便携标记 hash 均未改变；原数据目录存在但尚无 Vault，未自动创建加密库。

已启动替换后的原目录程序，5 秒后仍运行并创建 `Capstan` 主窗口，PID 为 33392。结果见 [`2026-10-14-replacement/report.json`](baselines/M1/raw/2026-10-14-replacement/report.json)。只验证替换完整性与启动，不把它计为原生认证或完整 A7 手测通过。维护者可直接在已打开的窗口继续原手测进度。

初始包含递归清理等步骤的整段操作被自动审批拒绝，仅返回 `blocked by policy`，没有执行。随后缩小为分步备份、单个 exe 复制和独立启动，全部成功；没有尝试递归删除或终止进程。`target/m1-replacement-20261014/` 留存已校验的新 exe，旧 exe 备份也保留以便回退。源码、PLAN 与既有 M1 验收状态不因本次替换改变。

## M1 终端高度回归修复（2026-10-13）

维护者在真实桌面手测中反馈终端只显示一行，截图确认下方大部分区域空白。原因是 M1 将标题栏移到 `.application` 后，内层 `.workspace` 只剩标签栏与终端两个子项，但仍沿用 M0 的 `36px / 34px / 1fr` 三行布局；终端因此进入固定 34px 的第二行，余下第三行无人使用。此前 jsdom/组件 mock 测试不能计算真实 CSS 布局，未检出此问题。

### 修改内容

- `ui/src/styles.css`：常规工作区改为标签栏 `34px` 与终端 `minmax(0, 1fr)` 两行；显式单列 `minmax(0, 1fr)` 与终端 shell 剩余行，避免内容最小尺寸撑出窗体。工作区最小尺寸约束集中在同一规则中。
- `ui/src/benchmark/BenchmarkApp.tsx`：使用独立 `.benchmark-workspace` 保留标题栏、工具栏、终端三行；benchmark shell 用明确的选择器保留说明行与内容行。未把常规工作区的两行套到基准窗口。
- 新增 `ui/tests/terminal-layout.html` 和 `scripts/check-terminal-layout.mjs`，由 `pnpm -C ui test:layout` 构建并测试实际生产 UI/xterm/CSS；使用随机的本机 HTTP 端口、独立 Edge/Chrome 临时 profile 与 loopback 调试端口，结束时校验 profile 绝对路径并清理。没有新增产品依赖。
- 更新本报告、M1-C、board 和 [重点手测指南](M1-priority-manual.md) 的当前包入口。已经创建的测试 Vault 可以在关闭应用后保留整个 `data` 目录，只更换 `Capstan.exe`，无需重新生成测试 SSH 账户或加密库。

### 测试结果

旧布局实测见 [`2026-10-13T13-15-17-718Z-terminal-layout/report.json`](baselines/M1/raw/2026-10-13T13-15-17-718Z-terminal-layout/report.json)：1600×1000 场景中 shell 高 34px、host 高 22px，xterm 行越出容器，与维护者截图一致。最终回归见 [`2026-10-13T13-24-46-442Z-terminal-layout/report.json`](baselines/M1/raw/2026-10-13T13-24-46-442Z-terminal-layout/report.json)，两种缩放共 24 项均通过。该场景修复后的 shell 高 930px、host 高 910px，终端网格高 882px；330×240 场景也保持可用的 150px host，没有行裁切或横向越出窗口。

布局矩阵包含 1600×1000、1024×768、800×360、600×480、330×240、恢复尺寸，新增/切换/关闭标签、搜索浮层，以及基准窗口的桌面/最小尺寸。每组分别在 100% 与 125% 缩放执行，核验实际 DOM 边界、xterm 网格适配和隐藏 pane 不占空间。Edge 文件版本 `156.0.4314.15`、Node `22.23.2`。Tabbit 注册不可用，使用新建的隔离 headless 浏览器测试；虚拟时钟未稳定调度 ResizeObserver 的中间运行保留失败记录，最终运行使用正常帧调度，没有通过放宽尺寸断言制造成功。

| 检查 | 结果 |
| --- | --- |
| `pnpm -C ui test:layout` | 24 项真实浏览器布局回归通过 |
| `pnpm -C ui typecheck` / `test` / 生产构建 | 通过；12 文件 / 58 项单元测试；JS 683.56 kB，gzip 192.92 kB，仍保留 chunk 大于 500 kB 警告 |
| `cargo fmt --all -- --check` / `cargo test --workspace --offline` | 通过；104 项，默认外部 SSH 测试仍按配置跳过 |
| `cargo test -p capstan-app --features tauri-runtime --offline` | 11 项通过 |
| 默认与 Tauri runtime 两组 Clippy | 均通过，`-D warnings` |
| 布局机械检查 / Node 脚本语法 | 通过；最终 CSS 结构验证以真实浏览器边界报告为准 |
| x64 NSIS / portable 构建与归档核对 | 通过；zip 五个条目及归档 exe 与实际 release exe SHA-256 一致 |

### 当前手测包与未决问题

当前使用 [`capstan-0.1.0-win-x64-m1-terminal-layout-20261013.zip`](../../target/bundle/portable/capstan-0.1.0-win-x64-m1-terminal-layout-20261013.zip)，6,103,406 字节，SHA-256 `FE7D1C287CCA858EC338C4F9E5F1DBABF81317DDEAB93C780EAE4DE7F42CF3C3`。归档 exe SHA-256 `93EF56D1744D98EB777294EF64DF5AD6EA0D5F7FF69C8A220D3BA5C28F3C081B`。包含本次修复的 NSIS 路径仍为 `target/release/bundle/nsis/Capstan_0.1.0_x64-setup.exe`，4,057,233 字节，SHA-256 `F4002E220A1CB1187E2D7C315179FF0A8EA6F3049FC8DB5A4041D766F4CD7E37`。下方旧包与 hash 是历史记录，以本节包为准。

关闭旧 Capstan，把新包解压到另一个目录，再将其中 `Capstan.exe` 复制到原便携目录替换旧 exe；保留原 `capstan.portable` 和完整 `data` 目录，重启后继续 P1/P2/P3。没有修改 Vault 格式或自动删除用户数据。

本轮浏览器测试不接 Tauri/PTY，不输入凭据，截图中的“桌面终端运行时不可用”为浏览器预览路径。因此只接受布局回归结果，不记为新的原生启动/认证/终端交互或完整 A7 通过。包仍未签名，Windows 原生完整手测、安装验收、性能基线及既有 RSA advisory 等待按前述契约收口；维护者只需用新包继续未完成的手测。`docs/PLAN.md` 未修改，SHA-256 仍为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。

## M1 自动联测与兼容性修复（2026-10-13）

本节记录真实后端联测；当前布局修复和手测包以上方最新节为准。此前的“真实 sshd 未配置 / Pageant 未联测 / KDF 无测量”仅描述当时状态。M1 总体验收仍未完成，不修改 PLAN 或把后端联测记为 A7 桌面闭环通过。

维护者当前优先执行 [M1-priority-manual.md](M1-priority-manual.md)：包含最新包校验、隔离临时 sshd 的准备/清理命令，以及 Vault 桌面闭环、Windows OpenSSH Agent 转发、真实换钥阻断三个流程和分项回报模板。它替代下方历史手测段落作为本轮操作入口，不扩大既有通过结论。

说明校验：7 个 PowerShell 代码块通过语法解析，2 个文件链接目标存在，最新包与 PLAN 的 SHA-256 匹配已有记录。准备命令复用下述已实测夹具；本轮说明校验不计为新增桌面手测通过。

### 修改内容

- 修复真实 OpenSSH 暴露的多 shell Agent forwarding 兼容性缺陷：开启转发时，第一条 shell 正常，第二条 shell 因重复 `auth-agent-req@openssh.com` 被拒绝。现在在同一 SSH 传输上以异步互斥保护首次请求，收到成功回复后只复用已有 Agent socket；转发授权仍是连接池隔离键，默认关闭。进程内服务端会拒绝重复请求，回归同时打开两个 shell 并验证只有一次转发请求、一次认证。
- 修复实际 Windows 启动暴露的致命接线缺陷：`main.rs` 再次调用 Tauri `.setup()`，覆盖 `register_commands` 中注册 Vault/信任状态的初始化；应用一启动便以 101 退出，报 `state() called before manage()`。窗口 Mica 与基准可见性现在与状态初始化共用唯一的 setup。新增 [`Test-M1Desktop.ps1`](../../scripts/Test-M1Desktop.ps1)，旧包重现失败，新 release exe 通过初始启动、移动后启动和阻塞数据路径拒绝启动三个进程/窗口回归。M0 benchmark 的 Clippy `map_or` 警告同时改为 `is_none_or`，基准 feature 的回归仍通过。
- 加固 `real_sshd` 测试：随机 UTF-8 探针通过八进制转义输出，必须匹配独立完整行，不能把 PTY 输入回显当成功；输出 EOF 后等待通道终态，再核验退出码。新增回显/部分匹配拒绝和 EOF/exit-status/close 时序回归。
- 新增 [`Test-M1Sshd.ps1`](../../scripts/Test-M1Sshd.ps1) 与 [`m1-sshd-fixture.sh`](../../scripts/m1-sshd-fixture.sh)。自动生成一次性账户、密码、Ed25519 私钥和主机密钥，在 WSL 的 `127.0.0.1:22292` 运行专用 sshd；没有 sshd 时解压发行版软件包执行，不安装或启用系统 SSH 服务，不修改 `/etc/ssh/sshd_config`、用户 `known_hosts` 或生产凭据。Windows 凭据目录在生成前设置仅当前用户继承 ACL。测试后关闭服务、移除临时账户和该次凭据；保存环境变量原值并还原。
- 脚本可用 `-PageantExecutable` 接入签名有效的官方 Pageant。发现既有 Pageant 时拒绝改变其身份；本轮只启动新测试进程、加载生成的测试密钥并按 PID 清理。新增真实远端签名和关闭→开启→再次关闭三次独立连接，以及专用服务端实际换钥后的错误码、旧/新对比和旧信任不被覆盖验证。
- 新增 [`measure_kdf.rs`](../../crates/capstan-vault/examples/measure_kdf.rs) 与 [`Measure-M1Kdf.ps1`](../../scripts/Measure-M1Kdf.ps1)。release 模式使用运行时随机口令、独立临时 Vault，记录五组策略内参数的 7 次解锁样本、一次预热和 OS 峰值工作集；不调整生产 KDF 参数，不生成真实 Vault 或记录秘密。

### 实际测试结果

真实联测原始记录：[`2026-10-13-201729-sshd-7b97b715de89/report.json`](baselines/M1/raw/2026-10-13-201729-sshd-7b97b715de89/report.json)。全部 8 个场景成功，`passed=true`、`cleaned_up=true`，没有把默认跳过分支计为联测。服务端为 WSL Ubuntu 24.04、`OpenSSH_9.6p1 Ubuntu-3ubuntu13.19` / OpenSSL 3.0.13；真实 Windows Pageant 为 Release 0.85，官方二进制 Authenticode 签名有效，签名人 Simon Tatham，SHA-256 `3BD76B57FCD09ACC948268C753EA2077B0C557CA435323301A9B572661F98ECD`。该程序仅保存在 `target/m1-test-tools/`，不纳入产品依赖或安装包。

| 场景 | 结果与边界 |
| --- | --- |
| 密码认证 | 真实 sshd 认证及两条 shell、UTF-8、100×32 resize、正常退出通过。 |
| OpenSSH Ed25519，未加密 / 加密 | 两种文件均由测试现场生成并成功连接；同样完成双 shell、resize、UTF-8 和退出。 |
| PKCS#8 Ed25519，未加密 / AES-256-CBC 加密 | 两种 OpenSSL 生成文件均成功连接；不扩大为所有 PEM 算法兼容结论。 |
| Windows Pageant Ed25519 | 真实身份选择和认证通过；关闭时无远端 Agent socket、枚举和签名失败；开启时远端 `ssh-add -T` 成功且双 shell 正常；再次关闭的新连接中枚举/签名再次失败。 |
| 保存临时主机信任 | 认证前内存信任已保存，专用测试信任文件在成功认证后保存供下一次进程验证，不触碰实际 Vault。 |
| 服务端换钥 | 实际重启专用 sshd 并换用另一把运行时主机密钥；返回 `host_key_changed`、对比新旧元数据、保留旧信任。认证前零请求仍由进程内计数测试证明，本项不以服务端日志计数。 |

| 工程检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test --workspace --offline` | 104 项通过；默认无外部目标的运行中仍会跳过外部测试，真实联测以上述专用报告为准 |
| `cargo test -p capstan-app --features tauri-runtime --offline` | 11 项通过 |
| 默认与 Tauri runtime 两组 Clippy，runtime `cargo check` | 通过 |
| `pnpm -C ui typecheck` / `test` / `build` | 通过，12 文件 / 58 项；JS 683.54 kB，gzip 192.92 kB，仍有超过 500 kB 的 chunk 警告 |
| `cargo deny check all` / SPDX 检查 | 通过；保留 RSA advisory 显式例外和重复依赖警告，不解释为无风险 |
| M0 工具回归 / PowerShell 语法 / fixture `sh -n` | 通过 |
| Tauri runtime + M0 benchmark feature 测试与 Clippy | 12 项通过，Clippy 通过；窗口可见性逻辑保留 |

KDF 初测保留在 [`2026-10-13-201319-kdf/report.json`](baselines/M1/raw/2026-10-13-201319-kdf/report.json)，构建检查结束后的第二次测量见 [`2026-10-13-202719-kdf/report.json`](baselines/M1/raw/2026-10-13-202719-kdf/report.json)：i7-14700HX、20 核 / 28 线程、Windows build 28020。默认 `65536 KiB / 3 / 1` 解锁中位数 237.92 ms、p95 257.81 ms，进程峰值工作集 69.57 MiB；最大内存 profile `262144 KiB / 4 / 1` 中位数 1305.28 ms、峰值 261.57 MiB。计时包含文件读取/锁与认证，内存包含初始化和预热；运行中的其他用户应用未关闭，因此作为参数评审数据，不能宣称已冻结性能阈值或获得完整应用性能结论。

### 桌面启动与当前手测包

旧包失败记录见 [`2026-10-13-203127-desktop/report.json`](baselines/M1/raw/2026-10-13-203127-desktop/report.json)，修复后 3 个场景成功记录见 [`2026-10-13-203151-desktop/report.json`](baselines/M1/raw/2026-10-13-203151-desktop/report.json)。便携数据目录确实被初始化、移动后仍能启动，未创建 Vault；数据路径被普通文件占用时非零退出，没有退回用户数据目录。阻塞文件场景不等于已实测只读 ACL/文件系统。验证过的 exe SHA-256 为 `41685014ADE540F3F3FCE4A09E08A3CBD7FF7EFCE6586F84D3B620B255525432`。

当前应使用包含两项产品修复的 [`capstan-0.1.0-win-x64-m1-sshd-desktop-20261013.zip`](../../target/bundle/portable/capstan-0.1.0-win-x64-m1-sshd-desktop-20261013.zip)，SHA-256 `3666B1201F3AE4FFFC413F30F033C18122C97D427CB41A7E443E7306BBDAC987`。较早的 `m1-verified-20261013.zip` 含启动缺陷，已移入 `target/bundle/portable/invalid/`；2026-10-12 和更早的 M1 包也不能用于本轮验收。新 zip 仍未签名，仅供隔离手测，不能作为发布候选。

zip 为 5,832,732 字节，已复核归档内 exe 与上述通过启动回归的 SHA-256 一致，以及便携标记、许可证、第三方声明和说明文件。包含启动修复的用户级 NSIS 已重新构建，位置 `target/release/bundle/nsis/Capstan_0.1.0_x64-setup.exe`，SHA-256 `71C59DDED55E82CC6D8FD4BCE575899492B2B8A5AACE451E9F4EEAFF6EB5B617`；尚未实际安装、升级或卸载，不记为 B7 完整通过。

computer-use 的截图与预期窗口内容不一致，随后用户用物理 Esc 停止了桌面自动操作；没有继续发送界面输入，也没有将截图作为 Capstan 渲染通过的证据。保留上述进程/窗口/目录回归结论，A7 完整视觉与认证交互仍未验收。被启动的测试应用已按其 PID 终止。

### 已知风险与未决问题

1. Windows OpenSSH Agent 服务仍保持测试前的 `Stopped / Disabled`，本轮没有提权、启用服务或用其他 Agent 冒充该服务。真实 Windows OpenSSH Agent 的成功、拒签与重启路径尚待单独验收。
2. 后端真实 sshd / Pageant / 换钥和启动崩溃修复已完成。A7 的桌面渲染、解锁、原生秘密录入、首次信任与重启持久化仍需维护者；Pageant 后端成功也不等于 GUI 复选框完整验收。computer-use 技能引用的 `guidance.md` 明确要求 “Do not automate user authentication dialogs.”，且用户已停止桌面自动操作；本轮只保留不含认证操作的启动回归。
3. 固定的 M0/M1 可见 WebView2 100 MiB 与三轮 20 标签比较、另一机器 `.cbak` 恢复、干净用户 NSIS 安装/升级/卸载和 ARM64 仍无验收结论。
4. [RustSec RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html) 于 2026-10-13 查询仍无 patched versions，继续阻断安全发布；本轮没有变更密码学实现或隐藏例外。
5. 一次失败运行的 Pageant 已按 PID 终止，WSL 测试服务和账户也已移除，但自动审批拒绝删除 Windows 临时凭据目录，工具仅给出 `blocked by policy`。`target/m1-sshd-56dbde153625/secrets/` 保留一次性测试密钥、口令和 PPK，受仅当前用户 ACL 限制；该身份已经没有对应测试服务器账户，目录仍须由维护者删除。后续成功运行的目录均已自动清理。不要将这个遗留目录作为证据上传、提交或打包。

未修改 `docs/PLAN.md`；SHA-256 仍为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。

## M1 历史收口复验（2026-10-12）

本节保留上一轮源码与测试结果；当前结论以上方 2026-10-13 自动联测记录为准。

- 已补齐 A 门实施记录 [`M1-A.md`](M1-A.md)。M1-C 的缺陷 1–10 在当前源码中均有对应回归；A1–A6 自动化通过。M0 原始手测目录 `baselines/M0/raw/2026-10-10-003852-manual-a54f29a2` 当前为空。维护者对剩余 M0 手测成功的报告保留为报告结论，但不能据此复核固定性能基线；board 中 M0 状态保持不变。
- `cargo fmt --all -- --check`、`cargo test --workspace --offline`（102 项）、Tauri runtime 测试（11 项）、默认与 Tauri runtime Clippy、Tauri runtime `cargo check`、`pnpm -C ui typecheck`、`pnpm -C ui test`（12 文件 / 58 项）、`pnpm -C ui build` 均通过。生产构建输出 59 modules，JS 683.54 kB（gzip 192.92 kB），仍有单 chunk 超过 500 kB 的警告。
- `cargo deny check all`、SPDX 许可证检查和 `scripts/Test-M0Tools.ps1` 通过。cargo-deny 检查使用仓库固定的 0.20.2；`RUSTSEC-2023-0071` 仍由 `deny.toml` 显式豁免，故“advisories ok”不能解释为无该漏洞。2026-10-12 查询到 RustSec `patched=[]`；`rsa 0.10.0-rc.19` 于 10 月 6 日发布的变更仅为兼容 `pkcs1`，未包含该漏洞的修复。没有自行修改密码学实现或弱化 RSA 能力来隐藏风险。
- `real_sshd` harness 共 3 项通过，目标未设置时打印 `SKIP: CAPSTAN_TEST_SSHD is not set; no external SSH server was contacted`。当前没有真实 sshd / Windows Agent 联测结论。
- 环境探测：Windows 有 `ssh.exe`/`ssh-keygen.exe`/`ssh-add.exe`，但没有 `sshd.exe`；`ssh-agent` 服务状态为 `Stopped / Disabled`。WSL Ubuntu 有 `ssh-keygen`/`ssh-add`，没有 `sshd`。因此用户手测前需要启动 Windows Agent 并准备一个专用真实 sshd（可在 WSL 安装 `openssh-server`，或使用已授权的测试服务器）。
- 以当前源码构建了未签名 Windows x64 NSIS 包：`target/release/bundle/nsis/Capstan_0.1.0_x64-setup.exe`，4,058,785 字节，SHA-256 `17A6997CDC70A85081AE1B930148A115C35EE9FB8F0FF8CC0D47A46AE3DF6F6C`。便携手测包为 [`capstan-0.1.0-win-x64-m1-agent-forwarding-20261012.zip`](../../target/bundle/portable/capstan-0.1.0-win-x64-m1-agent-forwarding-20261012.zip)，6,098,335 字节，SHA-256 `E8A498FB6CCA9A94DE3C689937F47BB775A1827225639B4AE1900D1CE15A0016`。已检查 zip 内容、19 字节便携标记及 PE `0x8664`（x64）；压缩包内 exe SHA-256 为 `E43E95F33BB653CEF6BFEB37481D9349145E92537F38A4F65131E4A7661491C0`。两包均未安装/启动验收，NSIS 未签名；便携包仅供隔离手测，不是发布候选。

M1 总体仍为 `blocked`：A7、维护者优先选定的 Agent forwarding 开/关手测、Pageant/真实 Agent 兼容、M0/M1 固定性能数据、安装/便携实机验收和 RSA advisory 均未收口。下一项由维护者执行的重点联测见本节之后的“维护者优先手测”；它使用上述最新 zip。未修改 `docs/PLAN.md`，SHA-256 保持 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。

## M1-00 前置项历史报告

实施日期：2026-10-09；测试说明补充：2026-10-10  
范围：按维护者补充指示，“按现有顺序先实现 M1-00 可自动完成的前置项，并在 M1-B.md 记录待手测阻断”。  
契约：[`M1.md`](M1.md) 第 2.2 节与 M1-00、[`M0.md`](M0.md)、[`PLAN.md`](../PLAN.md)。

## 交付结论

本轮补齐 Snap 原生命中代码、真实 PTY/WebView2 测量入口、原始报告验证和实际运行时包的只读核验工具，并补充回归测试。自动化范围及维护者本轮选定的 KEY-01～03 三项重点均已完成：可见基准五场景复核通过，默认终端输入/resize 和 5 标签隔离/关闭按维护者反馈通过。本轮不再要求追加手测。

完整设备基线、其余 Windows 体验及有效捆绑 ConPTY 按维护者指示暂缓，保留未测/工程缺口状态。完整 M0/M1-00 契约尚未签收，board 的总体 `blocked` 不代表本轮三项仍待测试。未进入 M1-01，没有实现 Vault、SSH、认证或连接池，也不具备 M1 A/B 门或 v0.1 发布结论。

未修改 PLAN.md。工作区没有 `.git`，无法给出真实 diff、提交 ID 或干净工作树结论；未提交或推送。当前方案文件 SHA-256：`2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。

## 修改内容

| 位置 | 修改与目的 |
| --- | --- |
| `vendor/tauri-plugin-snap-layout/` | 从 MIT 的 1.0.9 收窄出窗口适配器，使用原生子 HWND 返回 `WM_NCHITTEST / HTMAXBUTTON`，转发原生鼠标最大化/还原，保留键盘按钮路径；显式处理 owner thread、安装失败、子窗口销毁和父窗口属性清理 |
| `Cargo.toml`、`Cargo.lock`、`src-tauri/Cargo.toml` | 注册局部适配器补丁；新增依赖使用精确版本；测量 IPC、夹具和本地资源嵌入采用显式 feature |
| `src-tauri/capabilities/default.json`、`tauri.conf.json` | Snap 仅向 main 授权两条更新/解除命令，设置支持 Snap 的最小窗口尺寸；CSP 补齐框架内部 IPC 地址，应用和领域 crate 保留 `unsafe_code = deny` |
| `ui/src/window/`、`App.tsx`、`terminal/bridge.ts`、`terminal/contracts.ts`、i18n/CSS | 共享标题栏；用实际 DOM 边界与 DPR 同步命中区域，处理 resize、DPI/zoom、原生 hover、失败降级、迟到监听器和 StrictMode 所有权交接；文案提供中文/英文 |
| `ui/src/terminal/session.ts` | WebGL context loss 时释放 addon/监听器，重新可见时可重新挂载；加载失败释放已创建 addon；增加仅用于计数的解析完成回调，仍从 xterm.write 回调 ACK |
| `src-tauri/src/lib.rs`、`ui/src/terminal/frame.ts` 及测试 | 真实测量发现 CSP 的 JSON 回退；解码器明确拒绝 JSON 数组。Tauri 2.12.1 的小 Raw 帧也会内联成数组，在适配边界补零至 1024 字节强制二进制 fetch，解码验证补零；payload/校验/ACK/预算与共享 fixture 保留 |
| `crates/capstan-pty/src/benchmark.rs`、`src/bin/m0-fixture.rs` | feature 内的固定夹具，只接受 bulk/idle 两种模式；真实 ConPTY 读取边界记录字节、FNV-1a 和 EOF，不保留输出内容；夹具生成 100 MiB 并响应固定的六位探针 |
| `crates/capstan-terminal/src/service.rs` | 提供实时 pump 指标和已注册会话计数，用于最后一批 ACK 后及等待关闭后的资源核对；既有预算、EOF 和关闭语义保留 |
| `src-tauri/src/benchmark.rs`、`lib.rs`、`main.rs`、`tauri.m0-benchmark.conf.json` | 开发 feature 的受限命令与原始二进制 Channel 接线；main 来源、128 次夹具 admission 和严格报告 DTO；Rust 补充源端证据，固定路径、一次性同步报告写入；预检在桌面初始化前完成 |
| `ui/src/benchmark/`、`main.tsx` | 隔离的测量入口，执行 bulk、延迟 ACK、三轮 20 会话，复用实际控制器/会话/xterm；记录源/接收校验、ACK、响应样本、可见 renderer 和关闭结果；错误/取消仍回收已启动资源 |
| `scripts/Measure-M0.ps1`、`Verify-M0Report.ps1` | 实际进程树采样、元数据与 SHA-256、五轮证据验证；默认隐藏窗口，显式 `-Visible` 才显示；仅按本次 PID/启动时间清理，写证据失败也执行清理 |
| `scripts/Inspect-ConPtyBundle.ps1`、`Test-M0Tools.ps1` | PE 边界/架构、文件版本、签名与哈希核验；验证报告失败分支、恶意 PE 头及脚本语法；没有下载或捆绑实际运行时 |
| `package.json`、`ui/package.json`、`.gitignore`、`THIRD_PARTY_NOTICES.md` | 固定 pnpm 10.28.1、忽略适配器构建目录、保留 MIT 原始许可并记录直接依赖许可；不替代发布许可审计 |
| [`baselines/M0/README.md`](baselines/M0/README.md)、[`M1-00-manual-tests.md`](M1-00-manual-tests.md)、[`board.md`](board.md) | 可复现构建/采样步骤、详细手测用例和证据模板；区分自动化通过、实机检查与缺少入口的项目 |

没有启用通用 shell/fs 插件，领域 crate 没有引入 Tauri，终端输出仍为 Raw 二进制 Channel。测量报告不接受终端正文或任意路径，日志没有加入凭据或终端输出。

## 测试结果

测试环境：Windows x64，Rust/Cargo 1.96.0、Node 22.23.2、pnpm 10.28.1。workspace 声明 Rust 1.80，但实际锁定的 Tauri 2.12.1 与 tauri-plugin 2.7.1 的包清单均要求 Rust 1.90；当前桌面依赖不支持按声明的 1.80 构建，完整工程的最低工具链尚未验证。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 通过 |
| `cargo test --workspace --offline` | 41 通过：PTY 18、Windows 集成 3、共享终端 19、跨语言契约 1 |
| `cargo clippy --workspace --all-targets --features tauri-runtime --offline -- -D warnings` | 通过 |
| `cargo check -p capstan-app --features tauri-runtime --offline` | 通过 |
| `cargo clippy --workspace --all-targets --features m0-benchmarks,custom-protocol --offline -- -D warnings` | 通过 |
| `cargo test --workspace --features m0-benchmarks --offline` | 46 通过；额外覆盖源端滚动计数、真实夹具输入/resize/EOF、20 个真实 ConPTY 关闭等待、报告字段/样本边界和传输补零 |
| `cargo fmt --manifest-path vendor/tauri-plugin-snap-layout/Cargo.toml -- --check` | 通过 |
| `cargo clippy --manifest-path vendor/tauri-plugin-snap-layout/Cargo.toml --all-targets --offline -- -D warnings` | 通过 |
| `cargo test --manifest-path vendor/tauri-plugin-snap-layout/Cargo.toml --offline` | 4 通过：分数 DPI、恶意边界、负屏幕坐标、真实隐藏 HWND 的命中/复用/销毁与误 mouse-up |
| `pnpm -C ui typecheck` | 通过 |
| `pnpm -C ui test` | 8 文件、30 用例通过；新增 Snap 生命周期/失败分支、WebGL context loss、滚动校验及跨帧/ANSI 标记解析、JSON 拒绝/补零和内部 IPC CSP |
| `pnpm -C ui build -- --minify=false` | 56 modules，最终构建通过；JS 1,171.04 kB、gzip 224.97 kB |
| `pnpm -C ui build` | 较早一次压缩构建通过，出现超过 500 kB 的 bundle 提示；最终隐藏窗口测试使用上行非压缩构建 |
| `cargo build -p capstan-app --features m0-benchmarks,custom-protocol --offline`（测量配置覆盖）及固定夹具构建 | 通过；桌面 feature 的 `--m0-check` 预检可在创建窗口前返回 |
| `./scripts/Test-M0Tools.ps1` | 通过：有效合成契约、4 个报告拒绝分支、2 个无效 PE 分支，以及全部 PowerShell 脚本语法 |

编译器偶有增量缓存无法硬链接、改用复制的环境提示；退出码为 0，Clippy 检查无代码告警。前端工具需要访问已安装的 pnpm 缓存，资源采样需要本机 CIM，相关命令在本机权限环境执行。

### 自动化覆盖具体证明什么

- Rust 默认矩阵覆盖 shell/WSL 探测解析、运行时选择与失败回退、真实系统 ConPTY 启动/resize/退出，以及共享终端的合并、帧序号、ACK、预算、关闭等待和失败隔离。真实退出用例核对退出码及尾部输出，没有用单纯“进程启动成功”代表可交付。
- 测量 feature 的额外用例验证 PTY 读取边界计数/EOF、真实固定夹具的输入与 resize、20 个真实 ConPTY 的关闭等待、报告字段约束与 Tauri 传输补零；它不测试普通应用标签栏，也不启动真实 WSL。
- Snap 的四个测试包含真实隐藏 HWND 的命中/复用/销毁、负屏幕坐标和误 mouse-up；前端用例覆盖分数 DPR、StrictMode 所有权、迟到事件与订阅失败。这些不能证明真实 Windows 布局菜单、绘制与读屏通过。
- 前端检查覆盖跨帧与 ANSI 标记解析、解析回调 ACK、JSON 数组拒绝/合法补零、WebGL context loss 的释放与重挂载，以及输入和控制器生命周期。WebGL 测试使用可控接口，不代表设备驱动上下文实测。
- 工具正反例证明验证器会拒绝字节校验不一致、会话未回收、非二进制标记和缺场景，核验器会拒绝无效 PE；合成报告不是性能样本。
- 隐藏全链路另外验证实际 WebView2/Tauri IPC：五场景共 62 个真实 PTY，源/接收字节校验、parsed/ACK、延迟 ACK 暂停/恢复和资源计数吻合。输入样本只到 xterm 解析完成，不覆盖真实键盘、IME 和 GPU 呈现。

浏览器预览检查了默认尺寸与 330 × 240 最小尺寸的标题栏/终端布局。浏览器不具有 Tauri 桌面运行时，预览中的“桌面终端运行时不可用”是预期路径，不作为真实 PTY 或 Snap Layouts 验收。

### 隐藏 WebView2 功能证据

首轮证据保存在 [`2026-10-09-hidden-smoke`](baselines/M0/raw/2026-10-09-hidden-smoke/README.md)。五轮的源/接收字节和校验、解析/ACK 均一致，资源计数归零，但大帧的 `raw_binary = false`，验证器正确拒绝；这是失败证据。根据实际 Tauri 2.12.1 源码及 [官方 CSP 文档](https://v2.tauri.app/security/csp/) 修复内部协议许可，另外绕开框架小帧 JSON 优化并增加严格解码检查。

修复后的完整证据保存在 [`2026-10-09-hidden-final`](baselines/M0/raw/2026-10-09-hidden-final/README.md)。`Verify-M0Report.ps1` 通过五轮：`bulk:0` 耗时约 9.8 秒并完成 3 个探针，`delayed_ack:0` 暂停/恢复各 1 次，`tab_lifecycle:1..3` 每轮 20 个终端、60 个探针；所有轮次 `raw_binary=true`、源端/接收端字节与校验一致、解析/ACK 全部对齐、`renderer_peak=1`、关闭后 renderer/session 均为 0。请以 JSON 原始字段为准，避免把 ConPTY 转换后的约 106 MB 读取量误写成夹具生成量 100 MiB。该证据仍是隐藏窗口功能测试，不是可见设备性能基线。

## 已知风险

1. Snap 的自动化测试验证真实隐藏 HWND 的消息与资源所有权，未验证 Windows 11 实际布局菜单、屏幕绘制、读屏、高对比度和混合 DPI。原生子窗口透明绘制/命中策略需要真实桌面检查。
2. 测量夹具的 100 MiB 是 ConPTY 之前的生成量。原始 Channel 实测量由 PTY 读取边界计数；ConPTY 可能改变换行、VT 或屏幕输出。FNV-1a 是回归检测校验，不是安全认证。
3. 输入探针测量“提交到 xterm 解析回调”，不包含真实键盘、IME 和 GPU 呈现。20 会话基准使用实际底层会话/控制器，没有操作普通标签栏或真实 WSL；KEY-02/03 补充了默认输入/IME 与 5 标签的维护者反馈，不能扩大为完整普通 20 标签/WSL 验收。
4. WebGL addon 数为代码所有权计数，驱动资源仍需实机验证。离散系统采样可能漏过瞬时峰值和短生命周期进程；应用预算不是总 RSS 上限。
5. 实际 ConPTY 包、版本、架构、签名、再分发与 companion 文件仍未锁定。本轮使用系统实现；未知运行时版本保留 null，没有捆绑 DLL 或宣称 ARM64 实测。
6. 新窗口依赖保留局部 Win32 FFI，后续升级需单独运行适配器测试并审查销毁/重入。应用与领域代码没有放开 unsafe。
7. Cargo 离线解析新增窗口依赖时调整了部分传递依赖锁定值（包括 reqwest/hyper-util、rustix、wasm-bindgen 系列）；没有完整升级或发布许可审计。新增直接依赖版本固定，实际缓存清单中的许可为 MIT/Apache-2.0 或 MIT，原始插件许可保留。
8. workspace 的 Rust 1.80 声明低于已锁定桌面依赖的 Rust 1.90 要求，最低工具链需后续收口；本轮不修改 PLAN 或扩大到工具链治理。当前前端包含开发测量入口，构建体积提示仍存在。普通 Rust 构建不注册测量命令，不能据此断言 v0.1 打包与发行要求已经满足。
9. 小帧补零依赖实际锁定的 Tauri 2.12.1 的 1024 字节阈值。每个小帧最多增加 1004 字节传输开销；队列预算仍按终端 payload 计数，不包括框架、帧头和补零。框架升级需重验阈值及二进制实际路径。
10. 普通 UI 暂无 Shell/工作目录选择和内部队列/监听器/驱动上下文计数入口。默认标签只能使用探测所得本地 shell；嵌套 WSL 或 PowerShell 不能验收 Rust 的定向启动。这些是待补入口/测试的工程缺口，不能仅要求维护者在不存在的控件中操作。

## 未决问题与下一步签收

- 本轮 KEY-01～03 已收口；以下为维护者明确暂缓的完整验收范围，不是新的本轮操作要求。

- [ ] 维护者在哪台固定 Windows 11 设备、哪种 build profile 与 WebView2 版本上保存可见基线？需要记录显示器、DPI、字体与设备负载。
- [ ] 原生 Snap Layouts 的悬停/布局选择、点击与键盘操作、跨 DPI、Mica、可访问性是否通过？
- [ ] 常规应用三轮真实 20 标签（含可用的 PowerShell/WSL）与输出中关闭、启动取消、Channel 断开是否通过？
- [ ] 实际捆绑 ConPTY 选择哪个可再分发包、版本及 x64/ARM64 文件集合？完成许可/签名核验与有效选择/失败回退实测后才能签收。
- [ ] KEY-02 的默认中文 IME 已按反馈通过；完整 CJK/emoji/IME 显示矩阵、WSL 目录和发行版、真实 WebGL context loss/恢复仍暂缓。

## 维护者执行指示

**最新安排（2026-10-10）：** 维护者要求“先挑一点重点的，会影响到后面开发的测试我来完成，剩下的先不管”。本轮只执行 [`M1-00-priority-tests.md`](M1-00-priority-tests.md) 的三项：一次可见全链路、默认终端输入/resize、5 标签隔离与关闭。先获取会影响 SSH 复用终端基础的实机反馈；其余暂缓执行，保留未测/缺口状态。一次可见测量不冒充完整性能基线，5 标签不冒充三轮 20 标签；这不是 M0 全部签收或里程碑顺序变更，PLAN 未修改。

详细步骤集中在 [`M1-00-manual-tests.md`](M1-00-manual-tests.md)：每项包含操作、预期结果、证据、失败处理和未测边界，附完整重建命令、只读进程树采样及验收记录模板。

以下为完整验收的参考顺序，本轮暂不要求全量执行：

1. 第 2–3 节：使用独立基准版，在固定设备观察三组自动可见测量；程序负责五场景、校验、背压和探针。每组保存三份 JSON 与验证输出，不需要自己生成数据或计算性能分布。
2. 第 4–6 节：使用普通桌面版，检查 Snap/标题栏、键盘、IME、字体和 DPI，再真实操作三轮 20 标签及输出/启动中关闭。最后一个标签自动新建，关闭后的基线按一个会话记录；整窗关闭另测。
3. 第 7–8 节：记录 WSL/ConPTY/ARM64 等设备与入口缺口，并交回目录和结论。当前没有原生 WSL/工作目录选择控件或内部 GPU/监听器计数面板，相关入口和定向测试由代码侧补齐；不要求维护者猜数或把嵌套 shell 当成完整验收。

2026-10-10 已分别构建 `target/m0-manual/benchmark/`（app + 固定夹具）和 `target/m0-manual/desktop/`（普通 app），入口预检符合预期：基准版返回 `capstan-m0-benchmarks-v1`，普通版预检退出码为 2。普通桌面版使用 `tauri-runtime,custom-protocol`，不依赖 Vite。这些是可清理的本机构建产物，不是发行包。本轮重点反馈与实际可见证据见下方收口记录。

本轮没有提出或实施里程碑顺序、验收标准或 PLAN 决策变更；证据和工程缺口处理后再评估 M1-00 出口条件。维护者不必重复此前已完成的自动化矩阵。

### 测试说明补充验证（2026-10-10）

本次只更新测试说明、证据边界和 board，并准备独立本机入口；没有修改运行时源码或 PLAN.md。前述完整工程矩阵是 2026-10-09 的已通过结果，不重复记作本日新测试。

| 检查 | 实际结果 |
| --- | --- |
| 基准 app + 固定夹具构建、复制后预检 | 通过；使用测量配置与 `m0-benchmarks,custom-protocol` |
| 普通 app 本地资源构建、复制后预检 | 通过；使用空配置覆盖与 `tauri-runtime,custom-protocol`，预检退出码 2 符合预期 |
| PowerShell 文档命令 | 20 个代码块语法解析通过；不表示已经执行所有 GUI 操作 |
| Linux 命令示例 | Git Bash 的 `-n` 语法检查通过；没有启动 WSL，不能当作 WSL 兼容证据 |
| 普通应用资源采样示例 | 实际 PowerShell 根/隐藏子进程发现、子进程退出、PID 启动时间不符排除、零进程 JSON 保留及重新初始化防覆盖验证通过；检查产物在 `target/m0-documentation-checks/`，不是 Capstan 设备基线 |
| 本地文档链接与方案文件 | 本地链接存在性检查通过；PLAN.md SHA-256 保持原值，Cargo.lock 未变 |

完整原生 WSL、有效捆绑包及设备体验矩阵仍暂缓；单次可见测量、默认输入/resize 和 5 标签重点已按下方记录收口。

重点范围说明补充后，新增的 5 个 PowerShell 代码块语法检查通过；当前五份相关文档共 25 个 PowerShell 代码块、21 个本地链接检查通过，PLAN 与 Cargo.lock 哈希未变。本次仅调整测试执行优先级，没有新增或冒充维护者实机通过结果。

### 维护者重点反馈与证据复核（2026-10-10）

维护者提供 `2026-10-10-003852-manual-a54f29a2` 并反馈“剩下全部成功”。该目录为空；自动找到实际 [`2026-10-10-010106-priority-ca5bab70`](baselines/M0/raw/2026-10-10-010106-priority-ca5bab70/README.md) 的可见报告和验证输出，app/fixture 哈希与本轮准备的构建一致。没有移动或修改原始 JSON。

| 项目 | 结论 | 依据 |
| --- | --- | --- |
| KEY-01 可见全链路 | `accepted`（单次可见功能范围） | 复核五场景全部通过；62 个真实 PTY、186 探针，二进制/源端校验/ACK/EOF 正确；延迟 ACK 暂停/恢复各 1 次，关闭后会话/renderer 所有权计数为 0 |
| KEY-02 输入/resize | `accepted`（维护者反馈） | 本轮默认终端按键、方向键/退格、Ctrl+C、resize 与中文 IME 成功；未虚构截图或版本矩阵 |
| KEY-03 5 标签与关闭 | `accepted`（维护者反馈） | 本轮标签切换/隔离、输出中关闭、快速新建/关闭和整窗重开成功；没有扩大为三轮真实 20 标签或后台资源零残留证明 |

原始指标摘要与文件哈希见证据 README。可见 bulk 场景约 11.88 秒，提交到 xterm 解析的探针 p95 为 59.9 ms；没有预设性能阈值，不能单次测量即宣称性能达标。系统采样峰值为工作集求和 1298.94 MiB、私有字节 812.82 MiB、句柄 8106，存在一个未测进程条目；末样本在脚本清理前，不能当作关窗后残留。WebView2 为 156.0.4314.8，系统 ConPTY x86_64；显示缩放/是否远程桌面未单独提供，不阻断本轮反馈记录。

本轮重点已完成，不再追加维护者测试；此前暂缓清单仍按原状态保留。此次只复核证据和更新记录，没有运行时源码变更或完整工程矩阵重跑，PLAN 未修改。

## M1 自动实现与回归（2026-10-10）

本轮继续实现 M1 的可自动验证部分。Vault、SSH、Tauri 接线、前端日常流程与相应测试已经落地；A7 真实 OpenSSH 闭环、真实 Windows Agent/Pageant、发行交付和维护者验收仍未完成，因此 A/B 门和 M1 总体都不能标记通过，也不生成 v0.1 发布结论。

### 修改内容

| 区域 | 修改结果 |
| --- | --- |
| `crates/capstan-vault` | 增加版本化加密封装、有界头部/KDF 解析、数据密钥重包裹、schema 迁移、事务写入及最近 3 个已验证备份；新增改主密码、独立口令 `.cbak`、恢复和错误输入不覆盖原库的测试。恢复扫描限制为最多 128 个目录项及 8 次候选口令派生。 |
| `crates/capstan-core` | 增加 Vault 生命周期、主机/分组/凭据/偏好与信任数据接线；凭据保持在 Rust，导入私钥加密保存，引用文件在使用时重新读取和校验；备份合并预览按修订号与解锁代次绑定。主机只有引用 Agent 凭据时才能保存转发授权，转发策略也参与连接池键。新增认证恢复候选的短期不透明令牌、重复口令验证、过期失效和恢复后直接解锁；新增脱敏主机密钥列表和按指纹/修订号校验的显式移除操作。 |
| `crates/capstan-ssh` | 增加密码、公钥与 Agent 身份认证、首次信任先于认证、变化密钥阻断、可取消连接尝试、连接复用和 shell 租约；变化密钥会发送只读旧/新算法与指纹比较事件，然后返回阻断错误。Agent 转发仅对显式授权连接发送请求，使用有界双向 relay 连接所选本机 Agent，未授权通道会关闭；加入转发开/关进程内服务端测试。 |
| `crates/capstan-terminal` | 保留共享异步终端输入/输出、ACK、预算、EOF 与回收契约；增加 SSH 异步后端接入测试，证明一个饱和 shell 不阻塞同一连接的另一个 shell。 |
| `src-tauri` | 增加 Vault、主机、凭据、Agent、信任、改密、自动锁定和 `.cbak` 命令；通过原生文件选择与口令入口处理文件/口令；锁库会失效待确认操作和备份预览。恢复命令只接收 Rust 签发的短期候选 token，并在原生窗口重新收集口令。新增安全的 host-key DTO/list/remove 命令。增加便携标记/数据目录识别，启动时创建并写入、同步、删除探测文件，不可写则拒绝启动。 |
| `ui` | 增加主机/分组编辑、拖放、收藏、标签和搜索；密码/私钥数据不进入表单状态；增加 Agent 指纹选择、按主机启用的 Agent 转发开关（默认关闭，非 Agent 凭据不可开启）、改密和 `.cbak` 预览/合并 UI，以及终端搜索、OSC 52 写入确认/读取阻止和多行粘贴确认。解锁失败时可单独搜索已认证恢复文件，明确确认覆盖并在恢复时再次输入口令；UI 只保留不透明令牌、候选类型和修订号。新增 host-key 列表和需二次确认的移除入口；变化密钥弹窗显示旧/新算法与指纹，只有关闭按钮且继续阻断。 |
| `vendor/russh` | 移除 Agent 客户端会记录身份、公钥和签名数据的调试日志，并保留上游许可证及变更说明。 |
| `scripts/Build-Portable.ps1`、`src-tauri/src/data_directory.rs` | 便携包按 PE 架构输出 x64/ARM64 命名；便携模式只由 exe 同目录中的精确 `capstan.portable` 标记启用，Vault 放入同级 `data/`，其他情况使用 `%APPDATA%\Capstan`。临时打包目录限制在仓库 `target` 下，拒绝覆盖现有 zip，产物带 GPL、第三方声明和中英说明。 |
| `src-tauri/tauri.conf.json`、`scripts/Invoke-Tauri.ps1`、根 `package.json`、`ui/package.json`、lockfile | 固定 Tauri CLI 2.12.1；启用用户级 NSIS 目标，配置 UI 开发/发布命令，并提供从仓库根目录执行的 `pnpm bundle:nsis` 和 `pnpm portable:package`。便携入口先构建 no-bundle exe，防止复用被 NSIS 写入 bundle 类型标记的文件。 |
| `.github/workflows/windows.yml`、`deny.toml`、`scripts/Check-Licenses.ps1`、`LICENSE`、`THIRD_PARTY_NOTICES.md` | 增加 Windows x64 检查和打包工作流、固定 cargo-deny 0.20.2 的执行步骤、Rust/前端 SPDX 元数据检查及项目 GPL-3.0-or-later 全文。保留 RUSTSEC-2023-0071 的明确解释性例外并将其标为发布阻断；UI 三项额外许可证见第三方声明。 |
| `ui/vite.config.ts` | 在 Rollup 默认 tree-shaking 阶段反复增长内存并 OOM 后关闭 Rollup tree-shaking，保留 Vite 默认压缩。正式构建可重复完成；此规避增加未引用代码并触发 500 KB chunk 提示，发布前仍需定位工具链根因并复核包体。 |

未加入真实密钥或真实 Vault 测试夹具；自动化密钥及临时 Vault 在测试运行时产生。没有修改 `docs/PLAN.md`。

### 门与任务状态

| 范围 | 当前结论 | 仍缺内容 |
| --- | --- | --- |
| M1-00 | `blocked`（前置门） | M0 其余实机/交付条件仍按 board 保持原结论；维护者报告成功的 `2026-10-10-003852-manual-a54f29a2` 目录为空，不能据此补写可复核证据或更改 PLAN。 |
| M1-01～05 | `accepted`（自动化实现范围） | 尚无独立 M1-A 契约审查记录；A7 真实 WSL/OpenSSH、完整 KDF 设备校准与 M0 对比性能数据未完成。 |
| M1-06 | `needs-maintainer-manual` | A1–A6 自动化覆盖不替代 Windows/WebView2 解锁→指纹→认证→交互→重启恢复闭环。 |
| M1-07 | `implemented`（代码/测试范围） | CRUD、拖放、收藏、标签和搜索已接线；需在桌面应用实际检查焦点、排序、删除引用及重启行为。 |
| M1-08 | `needs-maintainer-manual`（自动化实现完成） | Windows OpenSSH Agent/Pageant 提供器、私钥引用/导入、主机级转发 UI 与 Rust 接线已实现；进程内 SSH 服务端验证授权后 relay 和默认关闭时拒绝远端主动通道，另有可选 `CAPSTAN_TEST_SSHD` harness。私钥自动化覆盖运行时生成的 Ed25519 OpenSSH/PKCS#8、加密/未加密和错口令路径；真实 Windows Agent/Pageant 与实际 OpenSSH/PEM/RSA 兼容矩阵仍待手测。Agent certificate 身份未支持。 |
| M1-09 | `accepted`（自动化范围） | 并发复用、隔离、取消、30 秒释放和慢通道覆盖进程内测试；真实服务器兼容性仍未测。 |
| M1-10 | `accepted`（自动化实现范围） | 改密、锁定、三份备份、导出预览/合并及认证恢复候选明确选择、恢复后解锁、错误口令保留主文件和过期 token 失效有测试；跨 Windows 用户/机器迁移与完整磁盘故障注入矩阵仍未完成。 |
| M1-11 | `implemented`（代码/测试范围） | 搜索、焦点恢复、OSC 52、多行粘贴和双语文案有 UI 测试；真实终端交互与安全提示仍待手测。 |
| M1-12 | `needs-maintainer-manual`（自动构建已补齐） | NSIS 与 x64 便携 zip 已本机构建，Windows CI 和许可证元数据检查已配置；尚未在隔离 Windows 用户中安装、升级、卸载或启动/移动/只读验证便携包，也没有远端 CI 运行记录。签名、ARM64 构建与发行审计仍未完成。 |
| M1-13 / M1 总体 | `blocked` | 可选真实 sshd harness 已实现但当前未配置目标并明确跳过；真实 OpenSSH、Agent/Pageant、性能基线、完整维护者清单及安装/便携包均未验收。 |

### 自动测试与工程检查

环境：Windows x64、Rust/Cargo 1.96.0、Node 22.23.2、pnpm 10.28.1。本次增量检查使用离线依赖，Rust 构建产物在仓库 `target/`；`CAPSTAN_TEST_SSHD` 未设置。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo fmt --manifest-path vendor/capstan-native-windows/Cargo.toml -- --check` | 通过 |
| `cargo fmt --manifest-path vendor/russh/Cargo.toml -- --check` | 通过 |
| `cargo test --workspace --offline` | 96 项通过：app 4、core 13、PTY/Windows runtime 21、SSH 20（17 个单元测试及 3 个 real-sshd harness 测试，目标未设时 1 项明确跳过）、terminal 20、Vault 17、共享契约 1。新增用例覆盖主机级 Agent 转发授权、转发连接池隔离、授权 relay 往返、关闭策略拒绝远端主动通道、私钥编码/口令矩阵及外部 sshd 配置边界。 |
| `cargo test -p capstan-app --features tauri-runtime --offline` | 11 项通过，覆盖备份预览令牌、锁库/自动锁定、连接尝试取消、断开选项、便携数据目录和二进制 Channel。 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 通过 |
| `cargo clippy --workspace --all-targets --features tauri-runtime --offline -- -D warnings` | 通过 |
| `cargo check -p capstan-app --features tauri-runtime --offline` | 通过 |
| `cargo test --manifest-path vendor/russh/Cargo.toml --no-default-features --features flate2,ring,rsa --offline --lib` | 138 项通过；功能组合与 `capstan-ssh` 实际启用项一致。单独按 vendored crate 默认特性构建时因本机缺少 NASM 未通过，故改用应用实际使用的 `ring,rsa` 组合验证。 |
| `cargo deny check all` | 前序 M1 实施检查退出码 0，advisories/bans/licenses/sources 四项均报告 `ok`；但 RUSTSEC-2023-0071 有下文明确说明的例外，不能作为“无已知漏洞”结论。此次增量环境没有安装 `cargo-deny`，未重跑该命令；本次许可证脚本仍通过。 |
| `scripts/Check-Licenses.ps1` | 通过；核对项目 GPL-3.0-or-later 文本哈希、383 个 Cargo 包及 231 个 pnpm 包的 SPDX 标识。 |
| `scripts/Test-M0Tools.ps1` | 通过；包括本次新增 PowerShell 打包/许可脚本语法检查。 |
| `pnpm -C ui typecheck` | 通过 |
| `pnpm -C ui test` | 12 个文件、51 项通过；新增主机 Agent 凭据选择、转发开关和保存回归。 |
| `pnpm -C ui build` | 通过；59 modules，JS 670.77 kB（gzip 189.12 kB），CSS 14.89 kB（gzip 4.16 kB）。仍有单 chunk 超过 500 kB 警告。 |
| `pnpm bundle:nsis` | 本轮重新构建通过；Windows x64 NSIS 安装器 `target/release/bundle/nsis/Capstan_0.1.0_x64-setup.exe`，3,943,513 字节，SHA-256 `FE027892A4D4E0955963F0BECB9D6D8103F043E75B75483E12E6532DDE637EC9`，未签名。只证明打包成功，未运行安装/升级/卸载验收。 |
| `pnpm tauri build --no-bundle --features tauri-runtime,custom-protocol` + `Build-Portable.ps1 -OutputPath target/bundle/portable/capstan-0.1.0-win-x64-agent-forwarding.zip` | 本轮通过；release exe 为 17,032,704 字节。新 x64 zip 为 5,654,345 字节，SHA-256 `C23C314083A14103301F6B9D1419515D7E73F00576ECFD56229ACE571E78515B`。核对含 exe、19 字节精确便携标记、GPL、第三方声明和双语说明；既有标准命名包及历史检查归档未覆盖。 |
| `pnpm tauri info` | 通过；识别 Tauri 2.12.1、Windows x64、WebView2 156.0.4314.8 及 app CSP。 |

`CAPSTAN_TEST_SSHD` 未设置，因此没有真实 sshd 兼容测试。Agent 转发自动化使用进程内 SSH 服务端和 TCP Agent 协议夹具，验证 SSH 通道授权、relay 和关闭路径；它不等同于 Windows OpenSSH Agent 或 Pageant 实机验收。NSIS 产物未安装运行；便携 zip 未启动或移动到只读位置测试。Windows CI 尚未由远端执行；当前构建产物未签名。

### 维护者优先手测（操作步骤沿用，包已更新）

当前仅优先做 Windows OpenSSH Agent 和 GUI 转发开关验收；真实 Pageant 后端已经通过。手测包更新为 [`capstan-0.1.0-win-x64-m1-sshd-desktop-20261013.zip`](../../target/bundle/portable/capstan-0.1.0-win-x64-m1-sshd-desktop-20261013.zip)，SHA-256 `3666B1201F3AE4FFFC413F30F033C18122C97D427CB41A7E443E7306BBDAC987`，包含启动与多 shell 转发修复，勿使用旧 M1 包。先在一次性 Windows 用户和专用 sshd 账户中使用它；不要连接生产主机、加载常用私钥或使用真实生产 Vault。本包未签名且只用于本地手测，不要作为正式安装包分发。

本机尚无 sshd：如果没有其他专用测试服务器，可在 WSL Ubuntu 安装 `openssh-server`（`sudo apt update` 后执行 `sudo apt install openssh-server`），保存 `/etc/ssh/sshd_config` 原件后为测试配置 `Port 2222`、`ListenAddress 127.0.0.1`、`AllowAgentForwarding yes`，创建专用账户和仅允许这把临时公钥的 `authorized_keys`。先用 `sudo sshd -t` 校验配置，再启动 sshd；Capstan 连接 `127.0.0.1:2222`。这样只监听 WSL 本机回环接口，不向局域网开放。若 Windows 经 localhost 转发无法连接，停止并记录结果，不要改成监听所有网卡。清理时恢复原配置、删除临时账户并停止仅为测试安装的服务。Windows `ssh-agent` 当前为 `Stopped / Disabled`，因此第 1 步会要求在管理员 PowerShell 临时启动服务并在结束时还原状态。

1. **准备一次性密钥和 Agent。** 先运行 `Get-Service ssh-agent | Select-Object Status,StartType`，并用 `Get-CimInstance Win32_Service -Filter "Name='ssh-agent'" | Select-Object State,StartMode` 记录原状态。若服务未运行，可从管理员 PowerShell 执行 `Set-Service -Name ssh-agent -StartupType Manual` 和 `Start-Service ssh-agent`；不要改变原本正在运行的服务。用 `ssh-keygen -t ed25519 -N "" -f (Join-Path $env:TEMP 'capstan-m1-forwarding')` 生成一次性测试钥匙，再运行 `ssh-add (Join-Path $env:TEMP 'capstan-m1-forwarding')` 和 `ssh-add -l`。列表只能有此测试身份。不要把私钥文件发给 Capstan UI 或复制给远端。
2. **准备专用 sshd 账户。** 把 `capstan-m1-forwarding.pub` 加入专用账户的 `authorized_keys`，并确认 sshd 允许 Agent forwarding，密钥行没有 `restrict` 或 `no-agent-forwarding`。把 `.pub` 文件另复制到远端账户家目录，路径为 `~/capstan-m1-forwarding.pub`，留给 `ssh-add -T` 做签名验证。使用专门主机指纹与测试账户，不要改生产 `authorized_keys`。
3. **验证关闭状态。** 解压上述 zip 到独立临时目录并运行 `Capstan.exe`，首次解锁只使用测试 Vault 主密码。新增该 sshd 主机，通过 UI 读取 Windows OpenSSH Agent 身份并保存 Agent 凭据。确认 Agent forwarding 默认关闭，选中 Agent 凭据后才可勾选。保持关闭并连接，在远端执行 `ssh-add -l`；预期提示没有可用 Agent 且返回失败。
4. **验证开启和签名。** 在主机编辑器勾选转发并保存，关闭该主机的所有旧标签，再重新连接。远端 `ssh-add -l` 应只列出该测试身份；`ssh-add -T ~/capstan-m1-forwarding.pub` 应返回成功。这样只证明远端可请求 Agent 签名，不会把本机私钥发送给远端。
5. **验证关闭后撤销和重启持久化。** 取消勾选并保存，关闭该主机的全部标签，再重新连接；`ssh-add -l` 与 `ssh-add -T ~/capstan-m1-forwarding.pub` 都应失败。用密码凭据打开另一主机编辑器时转发选项应不可用。重启 Capstan 后确认 Agent 凭据主机的开关状态按保存值恢复。策略修改仅适用于新连接，不能用既有转发 shell 验证撤销。
6. **清理并回报。** 从远端 `authorized_keys` 移除临时公钥并删除远端 `.pub` 文件；若使用临时 WSL sshd，恢复 `/etc/ssh/sshd_config` 原件、删除测试账户并停止测试服务。本机执行 `ssh-add -d (Join-Path $env:TEMP 'capstan-m1-forwarding')`，删除临时私钥和 `.pub` 文件，删除解压目录中的 `data/` 测试 Vault。若 Windows Agent 服务原先未运行，本次测试后停止它；将 `StartType/StartMode` 恢复到记录的原值（`Disabled`、`Manual` 或 `Automatic`）。只回报 Capstan zip SHA、Windows/OpenSSH/sshd 版本、关闭/开启/再次关闭三个结果和任何错误码；不要回传终端正文、私钥、公钥、口令或密码。若环境无法准备专用 sshd/Agent，记录具体缺失项并停止，不要用生产身份替代。

Pageant、完整 A7 信任闭环、`.cbak` 跨目录恢复、NSIS 安装/升级/卸载及便携启动/移动/只读检查列为后续维护者验收，不作为当前唯一优先项的前置条件。

### 已知风险与未决问题

- M0 总体仍按 [`board.md`](board.md) 保持 `blocked`；用户报告 M0 剩余手测成功，但其给定 raw 目录为空。既有 M0 priority 证据只覆盖该报告列明的范围，不补作整套 M0 验收。
- A 门实施记录现见 [`M1-A.md`](M1-A.md)，独立错误路径审查见 [`M1-C.md`](M1-C.md)；它们没有替代 `baselines/M1/` 原始性能数据或真实 Windows/sshd 证据。Vault 格式/恢复说明见 [`VAULT-FORMAT.md`](../VAULT-FORMAT.md)，SSH 认证兼容矩阵见 [`M1-SSH-COMPAT.md`](M1-SSH-COMPAT.md)。
- **安全发布阻断：** RUSTSEC-2023-0071 报告 russh 的 RSA 路径依赖 `rsa 0.10.0-rc.18` 存在 Marvin timing attack，当前没有可用上游修复。`deny.toml` 的单项例外只为保留可运行的审计入口；在替换/修复实现和覆盖 RSA 兼容性前，不可把该 RSA 路径作为安全可发布状态，也不可隐藏此例外。
- Agent forwarding 已有主机级授权、连接池隔离、默认关闭和进程内 relay/拒绝测试，但还没有 Windows OpenSSH Agent 与真实 sshd 联测；设置变更只作用于新建连接，已经打开的转发 shell 需要关闭才能撤销。Agent certificate 身份未支持，Pageant 也没有实机证据。
- 主机密钥变化 UI 已显示旧/新算法与指纹并保持连接阻断；独立管理允许在外部核验后移除旧信任，再由后续首次信任流程建立新记录。真实桌面手测尚未完成；导入信任冲突要求先核验 incoming fingerprint、取消预览、移除旧信任、重新预览后再导入。
- 便携运行与 `%APPDATA%` 数据隔离已有选择器、写入探测和打包命令，但只有自动化目录选择测试；须在隔离用户中验证便携首次创建、移动、只读/不可写错误，以及安装版升级/卸载保留 Vault。
- Rollup tree-shaking 关闭是临时兼容措施，当前压缩 JS 仍超过 500 kB。需定位 Rollup/模块图的内存增长，之后恢复 tree-shaking 或记录经审查的长期方案。
- M0 完整性能基线未定，M1 的 100 MiB、20 标签三轮、输入响应、资源回收和 KDF 校准没有测量结果，不对吞吐或内存作达标声明。
- 新增 Windows CI 定义尚无远端执行结果；NSIS/zip 均为未签名 x64 构建，WebView2 bootstrapper 和实际用户安装流程没有手测结论。ARM64 包仅预留了 PE 架构识别命名，没有构建或运行证据。
- 维护者报告 M0 剩余手测成功；指定 raw 目录为空，故设备和性能原始记录无法独立复核。本报告保留维护者报告，不据此更改 M0 状态、验收标准或 PLAN。

`docs/PLAN.md` SHA-256 仍为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。工作区未提供 `.git` 元数据，未报告 diff、提交或推送状态。

## Vault 恢复候选工作流补齐（2026-10-10）

补齐此前仅存在于 `capstan-vault` crate 的候选枚举/恢复 API：锁定态 UI 可以单独查找恢复文件，Rust 用原生窗口收集候选口令，只返回 token、类型和修订号。用户确认后再次通过原生窗口输入口令；Rust 检查 5 分钟 token、Vault 文件名前缀、候选内容摘要、修订号和完整 AEAD 认证，成功才原子替换主文件并保持解锁。失败认证、token 失效或内容被替换不覆盖主文件。历史密码保护的备份仍要求旧口令；文案已中英双语说明。具体格式与恢复步骤见 [`VAULT-FORMAT.md`](../VAULT-FORMAT.md)。

新增回归覆盖：损坏主文件不被错误密码覆盖；错误口令无法恢复；选定候选恢复后当前进程与重启均可解锁并读取对应版本；锁库后 token 失效；同目录其他 Vault 不能作为本 Vault 候选；枚举后被换成另一个有效文件的候选会因摘要不符而拒绝；UI 必须显式确认，传输中不含密码或路径。

| 本次检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test --workspace --offline` | 93 项通过；core 13、Vault 17，其余 app 4、PTY/Windows runtime 21、SSH 17、terminal 20、共享契约 1 |
| `cargo test -p capstan-app --features tauri-runtime --offline` | 11 项通过；恢复命令及原生凭据入口在 Tauri runtime feature 下编译 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 通过 |
| `cargo clippy --workspace --all-targets --features tauri-runtime --offline -- -D warnings` | 通过 |
| `pnpm -C ui typecheck` | 通过 |
| `pnpm -C ui test` | 12 个文件、56 项通过，含恢复候选确认/取消、错误口令、候选数量上限提示、脱敏信任列表及变化密钥只读对比 |
| `pnpm -C ui build` | 通过；59 modules，压缩 JS 678.76 kB、gzip 191.52 kB；仍有单 chunk 超 500 kB 警告 |
| `docs/PLAN.md` SHA-256 | 仍为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E` |

M1 的人工验收和发布阻断没有因此改变。恢复文件 UI 仍未在真实桌面损坏/恢复场景手测；跨用户 `.cbak` 迁移、完整磁盘故障注入、真实 sshd/Agent/Pageant、M1 性能基线、安装/便携验收和独立 M1-A/M1-C 审查仍未完成。RSA Marvin 风险继续阻断发布，M1 总体维持 `blocked`。本次未修改 `docs/PLAN.md`。

### 更新后的本机手测包

为本轮 Agent forwarding 优先手测，重新构建了包含恢复与主机信任管理改动的无签名 x64 便携包，保留先前归档：[`capstan-0.1.0-win-x64-m1-trust-management.zip`](../../target/bundle/portable/capstan-0.1.0-win-x64-m1-trust-management.zip)，5,731,731 字节，SHA-256 `2F482E0AF42EFD7B6F87244C018ED7E1112393FB986963867989BD1DDA92D02C`。压缩包内 PE 为 x64，包含便携标记、GPL、第三方声明和双语说明。只验证了构建及包内容，尚未启动/移动/只读位置测试。请解压到隔离的可写临时目录，按上文“维护者优先手测”的 1–5 步执行；该包创建的数据只位于解压目录的 `data/`。

## 主机信任管理与事务故障注入收口（2026-10-10）

新增独立 host-key 管理入口：展示目标、端口、算法、指纹和首次信任时间，不向 UI 返回公钥正文。移除旧信任需要浏览器确认，Tauri 命令同时校验目标、端口、当前指纹和 Vault 修订号，成功后新连接不会复用旧信任策略；既有连接继续运行。服务端提出已信任的不同密钥时，在完成认证前发送只读事件，UI 并排显示旧/新算法和指纹且只有关闭按钮。备份信任冲突继续阻断导入，并指导先通过可信渠道核验备份指纹、关闭预览、移除旧信任、重新预览。兼容边界见 [`M1-SSH-COMPAT.md`](M1-SSH-COMPAT.md)。

Vault 原子写入加入分阶段故障注入：临时文件创建、写入、flush、文件同步和 replace 前失败均保留完整旧文件并清理临时文件；replace 后的模拟失败则保留完整新文件。另有候选 KDF 派生次数上限、内容摘要防候选替换测试。完整磁盘、权限/空间不足和设备掉电仍未实测。

本轮最终回归为 `cargo test --workspace --offline` 93 项、Tauri runtime 11 项、UI 56 项通过；格式、两组 Clippy、Tauri runtime cargo check、UI typecheck 和生产构建均通过。`docs/PLAN.md` SHA-256 仍为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。

## 真实 OpenSSH 兼容测试入口（2026-10-10）

为补齐 M1 §7.2 和仓库测试约定的可执行入口，新增 [`crates/capstan-ssh/tests/real_sshd.rs`](../../crates/capstan-ssh/tests/real_sshd.rs)。它只在 `CAPSTAN_TEST_SSHD=user@host:port` 配置后访问外部 sshd；未设置时测试输出 `SKIP` 并且不连接网络。目标格式覆盖 DNS、IPv4 和带方括号的 IPv6，测试配置错误会失败而不会误报通过。

测试从专用密码文件、私钥/可选口令文件或已运行的 OpenSSH/Pageant Agent 读取认证材料；文件内容限制大小、按 UTF-8 读取并在 Rust 缓冲区清理。测试不会设置或核验这些文件的 ACL，也不会删除文件；维护者须使用一次性凭据、限制文件访问并在测试后清理。首次主机密钥只写入本测试创建的内存仓库，认证必须等该记录保存后才继续，不读写用户的 `known_hosts`。连接成功后打开同一传输上的两个 shell，验证 PTY resize、UTF-8 标记、独立通道及正常退出状态。若设置远端绝对 POSIX 公钥路径，则 Agent 认证连接启用转发，并在第二个 shell 执行 `ssh-add -T --` 验证签名；探针 shell 会先禁用 history 文件写入。

完整配置变量、边界和专用测试账户说明见 [`M1-SSH-COMPAT.md`](M1-SSH-COMPAT.md)。当前环境没有设置 `CAPSTAN_TEST_SSHD`，所以本轮只验证默认跳过分支和纯解析/转义测试；不能记为真实 OpenSSH、Windows Agent 或 Pageant 兼容通过。

本机探测到 Windows OpenSSH client `9.5.6.3`，但 Windows PATH 没有 `sshd`；WSL Ubuntu 也没有 `sshd`（只有 `ssh-keygen`），Windows `ssh-agent` 服务处于 stopped/disabled。因此没有自动启动或改动系统服务，也没有伪造外部 sshd 结果。

| 本轮检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test --workspace --offline` | 96 项通过；其中 SSH crate 17 个单元测试与 3 个真实 sshd 集成测试（目标未设时 1 项明确跳过，另 2 项解析/引用转义测试运行） |
| `cargo test -p capstan-ssh --offline -- --nocapture` | 17 个单元测试、3 个集成测试通过；输出明确显示 `CAPSTAN_TEST_SSHD` 未设置而跳过外部服务器连接 |
| `cargo test -p capstan-app --features tauri-runtime --offline` | 11 项通过 |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | 通过 |
| `cargo clippy --workspace --all-targets --features tauri-runtime --offline -- -D warnings` | 通过 |
| `cargo check -p capstan-app --features tauri-runtime --offline` | 通过 |
| `pnpm -C ui typecheck` | 通过 |
| `pnpm -C ui test` | 12 个文件、56 项通过 |
| `pnpm -C ui build` | 通过；59 modules，压缩 JS 678.76 kB、gzip 191.52 kB；保留超过 500 kB 的 chunk 警告 |
| `scripts/Check-Licenses.ps1` | 通过；核验 GPL 项目文本及 Rust/pnpm SPDX 元数据 |

本轮没有改变真实服务器测试结论：`CAPSTAN_TEST_SSHD` 未设置，A7 真实桌面闭环、Windows OpenSSH Agent 与 Pageant 实机、M1 对比性能、便携/安装验收及独立 M1-A/M1-C 审查仍未完成。维护者之前优先选择的 Agent forwarding 开/关 UI 手测仍按上文清单待记录；进程内夹具和本次外部测试入口都不能代替该结果。RSA Marvin 风险继续阻断发布。`docs/PLAN.md` SHA-256 仍为 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`，本轮未修改该文件。
