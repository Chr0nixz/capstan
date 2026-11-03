# M0 可复现测量与手测入口

M0 专属收尾与证据归属见 [manifest](raw/2026-10-17-closeout/manifest.json)；可见本地原始数据按哈希引用，和 M1 混合性能记录分别解释。

**最新状态（2026-10-17）：M0 已 accepted。** 官方运行时与原生 WSL 入口已补齐，当前设备可见本地参考 62 会话/186 探针、三轮 20 会话通过；[新证据与方法](../M1/PERFORMANCE.md)。Snap/Mica、CJK/emoji、DPI/多屏和 WebGL 恢复按维护者明确确认通过。以下暂缓/缺口描述保留历史参考，不是新的待测要求。

本工具属于 M1-00 的 M0 前置项收口。默认运行隐藏窗口，提供真实 ConPTY → 共享会话 → Tauri 原始二进制 Channel → xterm → ACK 的功能证据。可见窗口性能、真实应用标签、Snap Layouts 和有效捆绑运行时仍需维护者签收。工具通过不代表 M0 或 M1 通过。

本轮维护者只执行 [`三项重点手测`](../../M1-00-priority-tests.md)：一次可见全链路、输入/resize、5 标签隔离与关闭，其余暂缓。本页和完整说明中的全量验收步骤保留参考，本轮不要求全部执行。

三项重点现已完成，五场景报告复核及维护者输入/标签成功反馈见 [`本轮验收记录`](raw/2026-10-10-010106-priority-ca5bab70/README.md)。不需要继续本轮补测。

维护者实际执行请使用 [`M1-00 详细测试说明`](../../M1-00-manual-tests.md)：包含准备好的两种 exe 入口、三次可见基准、逐项操作/预期、失败记录、只读资源采样和验收模板。本页保留工具与指标说明；无需重复已完成的自动化矩阵。普通 UI 暂无 Shell/工作目录选择，嵌套 WSL 只能算交互测试，不能签收原生 WSL 启动。

## 构建

在仓库根目录的 PowerShell 中运行。需要当前工程使用的 Rust、Node、固定的 pnpm 10.28.1 和 WebView2；依赖未缓存时先按工程配置安装依赖。

```powershell
$ErrorActionPreference = 'Stop'
pnpm -C ui typecheck
if ($LASTEXITCODE -ne 0) { throw 'Frontend typecheck failed.' }
pnpm -C ui test
if ($LASTEXITCODE -ne 0) { throw 'Frontend tests failed.' }
pnpm -C ui build -- --minify=false
if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }

$previousConfig = $env:TAURI_CONFIG
try {
    $env:TAURI_CONFIG = Get-Content -LiteralPath src-tauri/tauri.m0-benchmark.conf.json -Raw
    cargo build -p capstan-app --features m0-benchmarks,custom-protocol --offline
    if ($LASTEXITCODE -ne 0) { throw 'Benchmark application build failed.' }
    cargo build -p capstan-pty --bin capstan-m0-fixture --features m0-benchmarks --offline
    if ($LASTEXITCODE -ne 0) { throw 'Benchmark fixture build failed.' }
} finally {
    $env:TAURI_CONFIG = $previousConfig
}
```

检查每条构建命令的退出码。`m0-benchmarks` 是显式开发 feature，普通桌面构建不注册测量命令，也不构建夹具程序。`custom-protocol` 使用本次构建的本地前端资源，避免依赖正在运行的 Vite 服务。配置覆盖只改变本次构建的窗口入口；常规配置文件仍使用普通终端页面。

两份可执行文件必须处在同一目录。`capstan-app.exe --m0-check` 在创建窗口前返回 `capstan-m0-benchmarks-v1`；普通构建返回退出码 2。该预检只确认 feature，使用上述配置覆盖才能进入测量页面。

## 运行与证据

每次使用新的输出目录，不覆盖历史证据。系统资源采样使用 Windows CIM，受限沙箱可能无法访问；在本机 PowerShell 中运行即可。

```powershell
./scripts/Measure-M0.ps1 `
    -OutputDirectory 'D:/Projects/capstan/docs/agent/baselines/M0/raw/my-hidden-run' `
    -TimeoutSeconds 600

./scripts/Verify-M0Report.ps1 `
    -ReportPath 'D:/Projects/capstan/docs/agent/baselines/M0/raw/my-hidden-run/terminal-report.json'
```

维护者执行可见窗口测量时，明确使用 `-Visible`，固定窗口尺寸、显示器、DPI、设备负载及构建 profile；前台保持该窗口，并记录这些条件。若要比较 release 构建，两条 Cargo 命令都加 `--release`，同时向采样脚本传入 `-Executable ./target/release/capstan-app.exe`。不要把 debug/隐藏窗口结果与 release/可见窗口吞吐直接比较。

```powershell
./scripts/Measure-M0.ps1 `
    -OutputDirectory 'D:/Projects/capstan/docs/agent/baselines/M0/raw/my-visible-run' `
    -Visible
```

| 文件 | 内容 |
| --- | --- |
| `terminal-report.json` | 每轮耗时、提交到 xterm 解析回调的输入响应样本、帧/字节/FNV-1a 校验、原始 ArrayBuffer 检查、ACK、renderer 数量及回收；Rust 补充 EOF、最终会话状态、实时流控和实际 ConPTY 选择 |
| `process-samples.json` | 固定间隔记录本次应用及可识别后代的工作集、私有字节、句柄、CPU 和文件版本；不采集进程命令行 |
| `environment.json` | OS、CPU、内存、显示适配器/驱动、测量时间、可见性、采样间隔、应用/夹具及锁文件 SHA-256 |

应用仅在 Rust 环境变量 `CAPSTAN_M0_REPORT_DIR` 指向已存在的绝对目录时启用测量，WebView 不提供文件路径或任意启动命令。报告 DTO 拒绝未知字段，只保存计数、固定错误码和合成输入的耗时，不记录终端内容。报告先同步到固定临时文件，再发布最终文件；单进程只接受一次发布。

脚本只追踪自己启动的进程树，以 PID 和启动时间识别进程；清理不按进程名称匹配。即使测量超时或证据写入失败，也执行已识别进程的清理。采样间隔内存活很短的夹具可能未被采样，PTY 回收另由会话关闭等待和真实进程测试证明。

失败和取消报告保留失败码，不包装成成功。只有五轮齐全且全部通过验证器，才是完整的功能测试记录。超时目录若仅含系统采样，属于失败证据，不能用于验收。

## 固定场景与指标边界

1. `bulk:0`：一个真实 PTY，夹具生成 100 MiB 带递增块标识的 ASCII/换行输出；三个合成输入探针。
2. `delayed_ack:0`：相同输出，先阻止 ACK，等待 Rust 记录暂停读取后恢复；验证暂停和恢复计数。
3. `tab_lifecycle:1..3`：每轮同时创建 20 个真实 PTY/xterm/controller，第一个持续输出，其余为固定探针夹具；依次切换可见性，收集每个终端的三个探针，再等待 EOF、关闭和资源回收。

每轮使用实际应用的会话、控制器、传输与 xterm 实现。生命周期场景没有操作常规应用的 Zustand 标签栏，也没有启动真实 WSL，因此不能代替 M0 B4 中 PowerShell/WSL 的真实标签手测。

- **100 MiB 是夹具在 ConPTY 之前生成的总量。** ConPTY 可能转换换行、VT 和屏幕输出。报告中的 Rust `digest` 在真实 PTY 读取边界计数，再与 WebView 的字节数和滚动校验值比较；不要把生成总量当作原始 Channel 的实测总量。
- 帧接收使用既有序号、长度和每帧校验协议；整体 FNV-1a 用于传输回归检测，不是密码学完整性保证。测量不保留输出正文。
- Tauri 2.12.1 会把不足 1024 字节的 Raw 帧转成内联 JS 数组。应用适配器仅在传输边界补零至 1024 字节，强制走二进制 fetch；前端验证补零并剔除，终端、校验、ACK 和预算只计实际 payload。共享核心编码与既有 fixture 不变。升级 Tauri 时必须重验此阈值；每个小帧最多增加 1004 字节传输开销。
- CSP 允许 Tauri 内部 `ipc:` / `http://ipc.localhost`，遵循 [Tauri CSP 文档](https://v2.tauri.app/security/csp/)。缺失该地址会触发框架的 postMessage/JSON 回退。前端解码器拒绝 JSON 数组并结束错误会话，不静默转换为字节。
- `input_response_ms` 从合成输入提交开始，到相应标记完成 xterm 解析回调结束，包含输入排队、PTY、IPC 和解析；不包含实际键盘输入、IME 或 GPU 呈现延迟。
- `renderer_peak` / `renderers_after` 是当前代码持有的 WebGL addon 数。WebGL 加载失败时保留默认 renderer；峰值为 0 不代表 WebGL 验收成功。驱动上下文及 GPU 资源仍需设备核验。
- `max_outstanding_bytes` 包含 queued 与 in-flight，允许越界一个最大 64 KiB 块。它不限制 ConPTY、WebView2、xterm 或整个进程树的 RSS。
- EOF 状态发布后仍可能收到最后的 ACK。Rust 报告读取实时 pump 指标，不直接用旧终态快照判断积压。
- 系统采样是离散值，不能宣称捕获瞬时峰值。WebView2 可能保留缓存；每轮关闭后要比较趋势和稳定值，不能把任何非零内存都视为泄漏或据此宣布没有泄漏。

## 自动化检查

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
cargo check -p capstan-app --features tauri-runtime --offline
cargo clippy --workspace --all-targets --features m0-benchmarks,custom-protocol --offline -- -D warnings
cargo test --workspace --features m0-benchmarks --offline
cargo fmt --manifest-path vendor/tauri-plugin-snap-layout/Cargo.toml -- --check
cargo clippy --manifest-path vendor/tauri-plugin-snap-layout/Cargo.toml --all-targets --offline -- -D warnings
cargo test --manifest-path vendor/tauri-plugin-snap-layout/Cargo.toml --offline
./scripts/Test-M0Tools.ps1
```

`Test-M0Tools.ps1` 只生成验证器的合成契约数据，测试拒绝校验不一致、未回收会话、非二进制标记和缺失场景；另验证实际运行时核验工具拒绝无效 DOS 头和越界 PE 偏移。此类文件在 `target/m0-tools-tests`，不属于设备测量。

## 实际捆绑 ConPTY 的核验入口

目前仓库没有 ConPTY DLL 或 OpenConsole 程序，实际测试使用系统 ConPTY。维护者选定合法可再分发的实际包后，可只读核验：

```powershell
./scripts/Inspect-ConPtyBundle.ps1 -RuntimeDirectory 'D:/reviewed-runtime/x64' |
    ConvertTo-Json -Depth 4
```

工具检查 PE 头边界、x64/ARM64 标识、文件版本、SHA-256 和 Authenticode。它不会下载、安装、复制或判定再分发授权；包来源、NuGet 版本、许可、签名与所有 companion 文件仍需人工复核。分别保存 x64/ARM64 结论；未知版本保留未知。

## 维护者验收清单

下列是契约验收范围。实际操作及缺入口/环境时如何记录，以详细说明为准；普通应用关闭最后一个标签会自动新建，标签循环应以一个会话为基线，整窗关闭另行验证。当前普通 UI 无驱动上下文、监听器与队列计数面板，不能靠任务管理器填写这些内部读数。

- [ ] Windows 11 普通桌面窗口悬停最大化按钮出现 Snap Layouts，选择布局有效；最大化/还原、键盘 Enter/Space、拖拽、最小化和关闭均正确。
- [ ] 100%/125%/150% 缩放及跨不同 DPI 显示器移动，原生命中区域始终对齐，按钮没有被子窗口遮住或留影；高对比度、读屏和 Mica 可用。
- [ ] 固定设备运行 `-Visible`，保存五轮完整报告和采样；记录 build profile、显示器分辨率/DPI、WebView2 版本、窗口尺寸和前台负载。
- [ ] 常规应用真实创建 20 个 PowerShell/WSL 标签，创建、切换、关闭至少三轮；检查 UI、进程、句柄、监听器和 GPU 上下文回收。
- [ ] 输出中关闭标签/窗口，启动中关闭，Channel 断开和 WebGL 上下文丢失后恢复，正常输入及尾部输出无异常。
- [ ] PowerShell 5.1/7、WSL 目录和发行版、中文输入法、CJK/emoji、字体回退与光标定位符合 M0 场景。
- [ ] 实际运行时包完成来源、版本、架构、签名、许可和 companion 核验，验证有效捆绑选择及各类失败回退。

签收结论填写到 board 和对应原始数据目录；未通过项保留复现步骤与风险。在这些前置项通过或维护者明确确认方案变更前，不进入 M1-01。
