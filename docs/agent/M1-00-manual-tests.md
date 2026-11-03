# M1-00 测试说明与维护者操作步骤

更新：2026-10-10。对应 [`M1-B.md`](M1-B.md)、[`M0.md`](M0.md) 和 [`M0 测量工具说明`](baselines/M0/README.md)。本说明不改变验收标准，不修改 PLAN.md，也不表示 M0 已签收。

**本轮执行范围已缩小：** 按维护者最新指示，只做 [`三项重点手测`](M1-00-priority-tests.md)：一次可见全链路、默认终端输入/resize、5 标签切换与关闭。下面的完整矩阵和长篇采样步骤保留作后续参考，本轮不要求执行。

**本轮反馈已收口：** 三项重点通过，见 [`实际证据与验收记录`](baselines/M0/raw/2026-10-10-010106-priority-ca5bab70/README.md)。无需追加本轮补测，暂缓清单仍保留原范围。

## 1. 先做哪些、哪些已经做完

你需要做的是：在实际 Windows 11 桌面观察可见基准，检查窗口/输入/显示体验，再操作普通应用的真实标签栏。基准中的输出、背压、校验、探针和资源计数由程序自动执行，不需要手工生成 100 MiB 或逐条核对字节。

| 范围 | 当前证据 | 维护者还需要做什么 |
| --- | --- | --- |
| Rust 核心、真实系统 ConPTY、关闭/EOF/背压 | 默认 41 个测试通过，测量 feature 下 46 个通过 | 无需重复运行这些测试 |
| Snap Win32 适配器 | 4 个测试通过，含真实隐藏 HWND 的命中/销毁 | 检查真正的 Snap 菜单、绘制、键盘和 DPI |
| 前端与脚本 | 30 个前端测试通过；类型检查、构建及脚本正反例通过 | 检查实际 WebView2 的输入、显示及标签操作 |
| 隐藏 WebView2 全链路 | 五个场景通过；共 62 个 PTY，源/接收校验、ACK、背压及关闭计数吻合 | 按第 3 节形成可见设备基线 |
| 原生 WSL、有效捆绑 ConPTY、真实 GPU context loss | 仍缺设备/包/入口证据 | 按第 7 节记录条件和缺口；不能用替代场景勾选通过 |

完整自动化命令和历史结果在 [`M1-B.md` 的测试结果](M1-B.md#测试结果)。隐藏运行原始数据在 [`hidden-final/README.md`](baselines/M0/raw/2026-10-09-hidden-final/README.md)。这些结果是 2026-10-09 的代码验证；本次文档更新另外验证了两种构建及入口，未宣称重新完成设备手测。

**完整验收时的参考顺序：** 第 2 节准备 → 第 3 节三次可见基准 → 第 4–6 节普通应用手测 → 第 7 节记录不可执行项 → 第 8 节回传证据。本轮只按重点手测说明执行；没有第二块显示器或 ARM64 设备时如实写“未测”。

## 2. 环境、可执行文件与记录目录

在应用外部打开 PowerShell，运行仓库命令；文中标注“Capstan 内”的命令才输入终端窗格。普通用户即可，不需要管理员。

```powershell
Set-Location 'D:/Projects/capstan'
$ErrorActionPreference = 'Stop'
$benchExe = 'D:/Projects/capstan/target/m0-manual/benchmark/capstan-app.exe'
$desktopExe = 'D:/Projects/capstan/target/m0-manual/desktop/capstan-app.exe'
$stamp = Get-Date -Format 'yyyy-MM-dd-HHmmss'
$suffix = [Guid]::NewGuid().ToString('N').Substring(0, 8)
$evidenceRoot = "D:/Projects/capstan/docs/agent/baselines/M0/raw/$stamp-manual-$suffix"
New-Item -ItemType Directory -Path $evidenceRoot | Out-Null
Write-Output "Evidence directory: $evidenceRoot"
```

已为本次工作区准备好下面两种 debug 构建，都嵌入本地前端资源，不要求启动 Vite。它们是 `target/` 下的本机构建产物；清理构建目录、换机器或修改源码后，按附录 A 重建。

| 文件 | 用途 | 启动后应看到 |
| --- | --- | --- |
| `target/m0-manual/benchmark/capstan-app.exe` | 固定五场景基准；与同目录 `capstan-m0-fixture.exe` 配套 | 通过采样脚本启动后显示测量进度，不是普通标签页 |
| `target/m0-manual/desktop/capstan-app.exe` | 普通应用体验手测 | 标题栏、一个终端标签、右侧 `+`，默认本地 PowerShell |

可以先运行不创建窗口的预检：

```powershell
$capability = & $benchExe --m0-check
if ($LASTEXITCODE -ne 0 -or $capability -ne 'capstan-m0-benchmarks-v1') {
    throw 'Benchmark executable preflight failed; rebuild with Appendix A.'
}
& $desktopExe --m0-check
if ($LASTEXITCODE -ne 2) {
    throw 'Desktop executable is not the intended normal build.'
}
```

普通版退出码 **2 是这个预检的预期结果**，没有窗口也正常。基准版返回固定字符串。不要直接双击基准版：采样脚本负责报告目录与可见性环境。

设备需具备 WebView2 Runtime。重建时，本次实际通过的工具链为 Rust/Cargo 1.96.0、Node 22.23.2、pnpm 10.28.1。workspace 虽声明 Rust 1.80，锁定的 Tauri 2.12.1 / tauri-plugin 2.7.1 已声明最低 Rust 1.90；**不能用 Rust 1.80 重建当前桌面依赖**。完整工程的最低工具链尚未单独验证。

开始前在证据目录创建 `operator-notes.md`，按第 8 节模板记录：测试日期、Windows、屏幕分辨率/缩放、字体是否安装、输入法、debug profile、是否插电、远程桌面/虚拟显示器以及后台高负载程序。OS、CPU、GPU、锁文件和 exe 哈希由脚本记录，通常无需手工抄写。

## 3. VIS-01：可见 WebView2 基准

### 操作

1. 使用固定显示器和缩放，关闭其他 Capstan 测试窗口。基准默认窗口逻辑尺寸为 1280 × 800；整个场景保持窗口尺寸、缩放、字体和设备负载条件一致。
2. 在外部 PowerShell 运行下面代码。每次会启动一个可见窗口、自动执行五场景、验证报告并关闭本次进程树；代码执行三次以保留重复测量。
3. 每次窗口出现后将它置于前台。观察输出和进度即可；不要调整尺寸、输入命令、点击取消或提前关窗。记录置前台之前的明显延迟；若运行中被其他窗口遮挡，保留本次证据并另测一组。

```powershell
for ($measurement = 1; $measurement -le 3; $measurement++) {
    $runDir = Join-Path $evidenceRoot "visible-$measurement"
    $logFile = Join-Path $evidenceRoot "visible-$measurement-validation.txt"
    ./scripts/Measure-M0.ps1 -Executable $benchExe -OutputDirectory $runDir -Visible -TimeoutSeconds 900 |
        Tee-Object -FilePath $logFile
    $report = Get-Content -LiteralPath (Join-Path $runDir 'terminal-report.json') -Raw | ConvertFrom-Json
    if (!$report.window_visible) { throw 'The report does not confirm a visible window.' }
}
```

目录名每次不同，不会覆盖旧证据。debug 构建的耗时不是发布性能承诺；历史隐藏运行约一分钟完成一整组，只作排错参考，不能作为本机通过阈值。

### 预期结果

每次验证输出应包含这五行，且没有异常；最后关于其他验收仍需手测的提示正常：

```text
bulk:0 : binary integrity, input samples, budget and cleanup verified
delayed_ack:0 : binary integrity, input samples, budget and cleanup verified
tab_lifecycle:1 : binary integrity, input samples, budget and cleanup verified
tab_lifecycle:2 : binary integrity, input samples, budget and cleanup verified
tab_lifecycle:3 : binary integrity, input samples, budget and cleanup verified
```

| 程序自动核验的项目 | 通过条件与含义 |
| --- | --- |
| 固定场景 | 每组 bulk、delayed_ack 各 1 个终端；生命周期三轮各 20 个 |
| 数据链路 | 所有 `raw_binary=true`；PTY 源端和 WebView 接收字节/FNV 相同；parsed/acked 与实际接收量相等 |
| EOF 与关闭 | 源端 EOF 为 true，最终 exited / 0；所有场景 `registered_after=0`、`renderers_after=0`，in-flight 为 0 |
| 背压 | 延迟 ACK 场景至少暂停/恢复各一次；最大未确认 payload 不超过 1 MiB + 64 KiB 的单块容差 |
| 输入探针 | 单终端至少 3 个样本；20 终端场景至少 60 个；测量提交到 xterm 解析回调的时间 |
| Renderer | peak 不超过 1；峰值为 0 也可能通过功能验证，需要另外记录 WebGL 是否实际创建 |
| 可见性 | 顶层 `window_visible=true`；它只证明程序报告窗口可见，不能证明始终前台、无遮挡 |

100 MiB 是夹具 **进入 ConPTY 前**的生成量。ConPTY 转换后的 Channel 字节数可以不同，不要求报告恰好等于 104857600；校验必须与实际 PTY 源一致。探针不测真实键盘、IME 或 GPU 呈现延迟，验证器也没有预设吞吐/延迟/总 RSS 达标阈值。

每个 `visible-N/` 必须有 `terminal-report.json`、`process-samples.json`、`environment.json`。验证日志在父目录。测量脚本结束后窗口消失是正常清理。需要再次验证已有报告时：

```powershell
./scripts/Verify-M0Report.ps1 -ReportPath (Join-Path $evidenceRoot 'visible-1/terminal-report.json')
```

WebView2 文件版本可从本次进程采样取出，不采集命令行：

```powershell
$samples = Get-Content -LiteralPath (Join-Path $evidenceRoot 'visible-1/process-samples.json') -Raw | ConvertFrom-Json
$samples.processes | Where-Object { $_.name -eq 'msedgewebview2' } |
    Select-Object name, version -Unique
```

没有采到版本时记录“未采到”，不要猜测。先把整组证据交回，我会从原始报告计算吞吐、响应分布、进程树峰值和三次差异；你不需要手工计算。

### 失败怎么处理

- 出现 `preflight failed` / `Build the application...`：入口错误，按附录 A 重建；没有开始成功测量。
- 提示输出目录已存在：重新执行第 2 节生成新目录。不要删掉旧证据后重测到同名目录。
- 五场景缺失、校验/ACK/预算/资源异常或超时：标记失败，保留目录、错误文本及当前进度；不要编辑 JSON 使验证器通过。
- 没出现测量窗口、显示“未启用”或连接页面错误：记录所用 exe 路径、启动命令和截图。无窗口的报告不能算 VIS-01 通过。
- 900 秒内未完成：脚本会尝试清理本次已识别进程，目录中的采样属于不完整证据。我来分析，不需要你杀掉所有同名 PowerShell/WebView2。

## 4. 普通应用启动与窗口手测

在外部 PowerShell 启动普通版，并保留 `$manualApp` 用于资源记录：

```powershell
$manualDir = Join-Path $evidenceRoot 'desktop'
New-Item -ItemType Directory -Path $manualDir | Out-Null
$manualApp = Start-Process -FilePath $desktopExe -WorkingDirectory (Split-Path $desktopExe) -WindowStyle Normal -PassThru
Get-FileHash -LiteralPath $desktopExe -Algorithm SHA256 |
    Format-List | Out-File -LiteralPath (Join-Path $manualDir 'executable-sha256.txt') -Encoding utf8
Write-Output "Capstan root PID: $($manualApp.Id)"
```

应进入普通标签页，不能仍是基准进度页，也不应显示“桌面终端运行时不可用”。本次没有实际捆绑 DLL，系统 ConPTY 是预期选择。

| ID | 具体操作 | 预期结果、需要记录 |
| --- | --- | --- |
| WIN-01 Snap 悬停 | 确认 Windows 开启 Snap 及最大化按钮悬停显示布局的选项；窗口处于还原状态。悬停右上角最大化按钮约 1–2 秒，选择一个布局区域，然后还原 | 出现系统 Snap Layouts；选区贴靠正确；按钮没有被透明/黑色原生块遮挡。若无菜单，用另一 Windows 应用对照系统设置并记录两者结果 |
| WIN-02 点击、拖拽 | 最大化/还原连续 5 次；拖动标题栏空白处；最小化后从任务栏恢复 | 每次只切换一次状态，拖动顺畅；按钮区域不会误触拖拽，拖拽区域不会误触按钮 |
| WIN-03 键盘按钮 | 点击 `+`，用 Shift+Tab 移动到最大化按钮的焦点轮廓；分别用 Enter、Space 最大化/还原 | 按键生效且没有双重切换；焦点轮廓可辨认。若无法移到按钮，记录焦点停在哪个元素 |
| WIN-04 DPI/多屏 | 在 Windows 的显示缩放设置中分别改为 100%、125%、150%（不是浏览器页面 zoom），重复 WIN-01/02；有多屏时跨不同 DPI 屏幕拖动，再悬停/点击 | 原生命中和图标对齐，无偏移、留影、遮挡；终端重新适配尺寸。单屏可测单屏缩放，跨屏项独立标未测 |
| WIN-05 最小尺寸/Mica | 拖小到最小尺寸附近（配置下限 330 × 240）；对比窗口激活/失焦，观察标题栏和内部终端 | 标题栏按钮可用，终端区域保持稳定深色表面；背景效果不破坏文字。记录系统透明效果/远程桌面条件，不能仅凭透明背景宣称 Mica 生效 |
| WIN-06 可访问性 | 测试机条件允许时，开启对比度主题、系统减少动画和讲述人，检查按钮名称、焦点和核心操作，再恢复个人设置 | 按钮能识别/操作，核心状态不只靠颜色；若未执行，逐项写未测，不能用 CSS 测试代替 |

WIN-01、04 失败时，截图包含按钮和系统布局菜单；记录屏幕分辨率、缩放、窗口最大化状态及是否远程桌面。不要为了本轮手测禁用显卡或改动系统 DLL。

## 5. 普通应用输入、显示和退出

下面的 PowerShell 命令输入 **Capstan 内**。先确认默认实际 shell 版本，标签名称不能替代实际版本：

```powershell
$PSVersionTable.PSVersion.ToString()
```

当前应用新标签只启动默认本地 shell（探测到 PowerShell 7 时优先选择它），没有 Shell/工作目录选择控件。

| ID | 操作步骤 | 预期结果 |
| --- | --- | --- |
| TERM-01 基础按键 | 手动输入 `Write-Output 'CAPSTAN_TEST_OK'` 回车；按 ↑ 调历史，再用 ←/→ 和 Backspace 修改文本后运行 | 回显正确、方向键和退格不丢字/重放，提示符正常返回 |
| TERM-02 Ctrl+C | 运行下方持续输出命令，2–3 秒后按 Ctrl+C，再运行 TERM-01 | 输出停止并返回提示符；后续命令可正常执行；应用不挂起 |
| TERM-03 Resize | 运行尺寸命令，横向/纵向调整窗口，再运行一次；在窄窗输入较长的一行，再放大 | 列/行数随实际终端变化；自动换行与光标定位正确，没有重复/漏字 |
| TERM-04 ANSI/Unicode | 运行下方颜色及 Unicode 命令，在含中英文/emoji/组合字符的输入行用方向键、退格编辑 | ANSI 不显示为控制符，颜色复位；中文不乱码，宽字符/emoji 后的光标位置与内容一致。组合 emoji 的具体图形可随字体变化，需记录字体与对齐结果 |
| TERM-05 中文 IME | 在提示符手动输入 `Write-Output '`，切中文输入法输入“中文测试”但先不选候选；移动候选选择、取消一次、再提交，补 `'` 回车。窄窗换行后与 DPI 改变后各重复一次 | 候选框靠近当前光标，预编辑不提前发到 shell；取消不留残字，最终只提交一次正确文本；换行/DPI 后仍定位正常 |
| TERM-06 隐藏/恢复 | 一个标签运行持续输出，切到另一个标签执行 TERM-01，再切回；循环至少 20 次 | 可见终端持续渲染，返回原标签仍可输入；没有黑屏、旧画面或“一次按键输入两次”。这项观察不等于实际 GPU context loss 验收 |
| TERM-07 自然退出 | 在非最后一个标签运行 `exit 7`，观察终态，关闭此标签，再点 `+` | 退出消息包含实际退出码 7，旧会话不继续接收输入；新标签仍可启动。关闭最后一个标签会自动新建一个，应用不会保持零标签 |

持续输出（约 100 秒有界运行，可 Ctrl+C 中止；用于 TERM-02/06 与关闭测试）：

```powershell
1..1000 | ForEach-Object { Write-Output ('tick {0:D4}' -f $_); Start-Sleep -Milliseconds 100 }
```

尺寸、颜色和字符（分别执行）：

```powershell
'cols={0}, rows={1}' -f [Console]::WindowWidth, [Console]::WindowHeight
$esc = [char]27
Write-Output ("${esc}[31mRED${esc}[0m NORMAL")
Write-Output 'ASCII|中文|汉字|🙂|é|END'
```

中文 IME 必须真正用输入法输入；粘贴中文只验证字节/显示，不能算 IME 通过。截图只使用这些合成测试文本，不开启终端全文 transcript。

## 6. TAB-01～03：真实标签栏与关闭回收

### TAB-01 三轮 20 标签

1. 先恢复至 **一个**运行中的标签。点标签右侧 `×` 会关闭该标签；关到最后一个时会自动新建，因此基线是一个会话，不是零会话。
2. 在外部 PowerShell 按附录 B 初始化资源记录，执行 `Save-M0ManualSample 'initial-one-tab'`，等待 30 秒后再采样一次。
3. 在 Capstan 点 `+` 19 次，总数为 20。横向滚动标签栏访问被挤出屏幕的标签；每个标签等到提示符出现。若已有别的数量，先回到一个再计数。
4. 在一个标签启动第 5 节持续输出命令。按顺序切换所有标签，在其他标签运行 `Write-Output 'CAPSTAN_TAB_OK'`。检查输出归属、选中状态、焦点、横向滚动和新建/关闭按钮；切回输出标签时应继续看到内容。
5. 采样 `Save-M0ManualSample 'round-1-twenty-tabs'`。关闭 19 个标签，只留一个静止标签；可以直接关闭正在输出的标签，不必先 Ctrl+C。
6. 等待 30 秒，采样 `Save-M0ManualSample 'round-1-one-tab-after-30s'`，在剩余标签运行 TERM-01，确认仍可操作。
7. 总共重复 3 轮，采样名称改为 `round-2-...` 和 `round-3-...`；每轮重新回到 1 个再加 19 个。

预期：每个可见标签能输入/渲染，隐藏会话不串输出，关闭后对应 shell 回收，剩余标签仍可用。句柄、私有字节/工作集形成可比较的稳定样本，不要求缓存严格恢复到启动值。若三轮后持续单调增长或有旧 shell 残留，保留样本并标为待分析，不凭目测直接判定无泄漏。

当前没有普通应用的 WebGL 驱动上下文、输入监听器、实时队列读数面板。附录 B 只测已识别进程树；**这几类内部指标填写“普通 UI 无观测入口”**，对应自动化所有权/预算证据另列，不要求你从任务管理器猜数。仍需真实 GPU/普通 UI 指标时由我补受限观测入口。

### TAB-02 输出中关闭、启动中关闭

- 新建标签，运行持续输出后立刻点此标签 `×`。观察其他标签可以继续输入；30 秒后采样 `output-tab-closed-after-30s`，核对新增 shell 是否结束。
- 连续做 5 次“点 `+` 后立即点新标签 `×`”，覆盖尚未出现提示符时关闭；30 秒后采样 `startup-cancel-after-30s`。预期没有迟到输出进入其他标签，也没有逐次增加的旧 shell。
- 标签关闭可能发生得太慢，5 次都已出现提示符时，要写“快速关闭通过，未命中启动阶段”；不要据此宣称启动取消实机通过。

### TAB-03 整窗关闭

1. 在一个标签持续输出，再创建一个标签，点击右上角关闭按钮。
2. 等待 30 秒，在外部 PowerShell 执行 `Save-M0ManualSample 'window-closed-after-30s'`。
3. 预期本次根进程及已观测的 shell/conhost/WebView2 后代结束。若有残留，记录 PID、名称、启动时间和该阶段样本；不要按名称结束其他应用的 WebView2/PowerShell。
4. 再启动普通版确认能够新建终端。若继续采样，先创建新的普通应用证据子目录并更新 `$manualDir`、`$manualApp`，再初始化附录 B；保留前一次 JSON，避免沿用旧根 PID 或覆盖证据。

采样有离散间隔，不能发现所有短命进程，也不能证明 Rust 内部任务数为零。代码任务归零由已有自动化测试证明；实机残留由本节另行记录。

## 7. 可选环境与当前不能直接完成的项目

| 项目 | 你现在可以怎么测/提供信息 | 证据边界及后续负责方 |
| --- | --- | --- |
| PowerShell 5.1/7 | 记录默认实际版本。另一版本已安装时，可在 Capstan 中启动 `powershell.exe -NoLogo -NoProfile` 或 `pwsh.exe -NoLogo -NoProfile`，重复 TERM-01～05，再 `exit` 返回外层 | 嵌套 shell 验证输入/显示，不能证明 Rust 按 `shell_id` 直接选中该版本。缺版本标未测；不必为本轮安装软件 |
| WSL 交互 | 有 WSL 时按下方步骤进入一个实际发行版，检查输入/Unicode/resize；记录发行版名称/WSL 版本 | 当前 UI 不提供原生 WSL 标签/工作目录选择；嵌套 `wsl.exe` 不能验收 Rust 原生 WSL 启动、默认值、Linux 工作目录或注册表失败回退。这些缺口由我补入口/定向测试 |
| 真实 GPU context loss、Channel 故障 | 做 TERM-06 并记录黑屏/恢复异常；提供 GPU/驱动/WebView2 条件 | context loss、监听器销毁及 Channel 失败已有自动化覆盖；真实故障注入仍需专门入口。无需自行关闭显卡或杀系统 WebView2 |
| 有效捆绑 ConPTY | 如已有候选包，提供包路径或来源、版本及许可文件；只读核验命令见下文 | PE/版本/哈希/签名、包清单核验及失败回退自动测试由我完成；没有实际包不能勾选有效捆绑通过 |
| ARM64/多屏/读屏条件 | 记录是否有设备/环境和已执行的场景 | x64 检查 ARM64 PE 头不等于 ARM64 运行。缺环境写未测，另行安排，不能用交叉编译代替 |

WSL 已安装时，在 **外部 PowerShell** 获取可用发行版，不安装/修改系统：

```powershell
wsl.exe --version
wsl.exe --list --quiet
```

旧版 WSL 若不支持 `--version`，保存该事实和系统版本，仍可尝试列出已有发行版。在 **Capstan 内** 输入下例，将发行版名称替换为上一步实际名称（示例不是固定要求）：

```powershell
wsl.exe -d 'Ubuntu'
```

进入 Linux shell 后执行，调整窗口后再次运行 `stty size`，最后 `exit` 返回 PowerShell：

```bash
printf '%s\n' 'WSL_TEST_OK' '中文|🙂|END'
pwd
stty size
exit
```

预期无乱码，Linux 输出属于当前标签，`stty size` 行/列随窗口变化，退出后外层 PowerShell 可继续输入。报告项名称写 **“嵌套 WSL 交互”**；原生 WSL 标签和初始工作目录保持未验证。

如已有候选 ConPTY 包，可只读核验一个真实存在的目录；替换示例路径：

```powershell
$candidateDirectory = 'D:/reviewed-runtime/x64'
./scripts/Inspect-ConPtyBundle.ps1 -RuntimeDirectory $candidateDirectory |
    ConvertTo-Json -Depth 4 | Out-File -LiteralPath (Join-Path $evidenceRoot 'conpty-inspection.json') -Encoding utf8
```

这个脚本不会下载、安装或证明再分发授权。`Valid` 签名不等于许可核验通过，未知版本写未知。当前 runtime 加载目录是 **exe 同级 `runtime/x86_64/conpty.dll`（x64）或 `runtime/aarch64/conpty.dll`（ARM64）**；不是随意放在工作目录或名为 `runtime/x64` 的目录。准备的两份 exe 各有独立运行时目录；复制 exe 不会自动复制候选包。

包核验后再由我安排完整 companion 文件和有效选择/缺文件/错架构/坏 DLL/创建失败回退的定向验证。先不要在应用目录混放任意来源 DLL。缺包时记录“未提供候选包”，这不是你操作失败。

## 8. 如何记录和交回

把以下模板复制到 `$evidenceRoot/operator-notes.md`。每个 ID 写 `通过`、`失败`、`未测` 或 `无法执行（缺入口/环境）`；空白不能当作通过。耗时/资源精确数由 JSON 和采样支持，不需要手抄全部字段。

```markdown
# M1-00 device acceptance

- Date / operator:
- Evidence directory:
- Build profile / desktop exe SHA-256:
- Windows / WebView2:
- Monitor resolution / scaling / window size:
- GPU / driver / local or remote desktop:
- PowerShell default version / WSL distributions:
- Fonts / IME:
- Power state / competing workload:

| Test ID | Result | Actual behavior / evidence / limitation |
| --- | --- | --- |
| VIS-01, repetitions 1–3 | Pending | |
| WIN-01–06 (record individually) | Pending | |
| TERM-01–07 (record individually) | Pending | |
| TAB-01, rounds 1–3 | Pending | |
| TAB-02 output / startup close | Pending | |
| TAB-03 window close | Pending | |
| Nested WSL / other PowerShell | Pending | |
| Native WSL / working directory | Pending | |
| Bundled ConPTY / ARM64 | Pending | |
| GPU context loss / internal UI counters | Pending | |

## Failure record
- Test ID and time:
- Starting tab count / shell / DPI / window state:
- Exact actions:
- Expected result:
- Actual result / error code:
- Frequency (for example 3 of 5 attempts):
- Report / screenshot / sample file:
```

交回整个目录路径即可：三组可见 JSON、验证日志、普通应用资源 JSON、exe 哈希、手测记录和必要的合成数据截图。若脚本失败，另附异常文本；不需要发送真实凭据、终端全文或个人文件内容。我会复核数据、定位失败并更新 M1-B/board 的实际结论；缺入口的项目保持明确缺口，不把它们转嫁为“请维护者自行证明”。

## 附录 A：清理 target 或源码变化后重建两种入口

这是重建步骤，不是要求维护者重新运行自动化矩阵。所有命令都在仓库根目录；`--offline` 适用于当前已缓存依赖。换机器缺缓存时，先正常安装依赖并构建，不能把离线依赖缺失记成代码测试通过。

```powershell
Set-Location 'D:/Projects/capstan'
$ErrorActionPreference = 'Stop'
pnpm -C ui build -- --minify=false
if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
$previousConfig = $env:TAURI_CONFIG
try {
    $env:TAURI_CONFIG = Get-Content -LiteralPath src-tauri/tauri.m0-benchmark.conf.json -Raw
    cargo build -p capstan-app --features m0-benchmarks,custom-protocol --offline
    if ($LASTEXITCODE -ne 0) { throw 'Benchmark application build failed.' }
    cargo build -p capstan-pty --bin capstan-m0-fixture --features m0-benchmarks --offline
    if ($LASTEXITCODE -ne 0) { throw 'Benchmark fixture build failed.' }
    $benchmarkDirectory = 'D:/Projects/capstan/target/m0-manual/benchmark'
    New-Item -ItemType Directory -Path $benchmarkDirectory -Force | Out-Null
    Copy-Item -LiteralPath target/debug/capstan-app.exe -Destination (Join-Path $benchmarkDirectory 'capstan-app.exe')
    Copy-Item -LiteralPath target/debug/capstan-m0-fixture.exe -Destination (Join-Path $benchmarkDirectory 'capstan-m0-fixture.exe')

    $env:TAURI_CONFIG = '{}'
    cargo build -p capstan-app --features tauri-runtime,custom-protocol --offline
    if ($LASTEXITCODE -ne 0) { throw 'Desktop application build failed.' }
    $desktopDirectory = 'D:/Projects/capstan/target/m0-manual/desktop'
    New-Item -ItemType Directory -Path $desktopDirectory -Force | Out-Null
    Copy-Item -LiteralPath target/debug/capstan-app.exe -Destination (Join-Path $desktopDirectory 'capstan-app.exe')
} finally {
    $env:TAURI_CONFIG = $previousConfig
}
```

`TAURI_CONFIG='{}'` 为普通构建提供空覆盖，避免继承基准页面配置；最后恢复原环境。`custom-protocol` 必须保留以嵌入资源。两种构建都会改写 `target/debug/capstan-app.exe`，所以复制后使用各自的 `target/m0-manual/` 入口。关闭正在使用的测试 exe 后再重建；不要覆盖运行中的文件。

## 附录 B：普通应用的只读进程树资源采样

在第 4 节启动应用的同一个 **外部 PowerShell** 中粘贴整个代码块。只采样本次根 PID 和发现的后代，不记录命令行/终端内容，不结束进程。PID 与启动时间配对，避免把已复用的 PID 当成旧资源。每次新启动一份应用要重新初始化。

```powershell
$manualSamplesPath = Join-Path $manualDir 'process-samples.json'
if (Test-Path -LiteralPath $manualSamplesPath) {
    throw 'Use a fresh manual evidence directory when initializing a new process tree.'
}
$manualBirths = [Collections.Generic.Dictionary[int, long]]::new()
$manualBirths.Add($manualApp.Id, $manualApp.StartTime.ToUniversalTime().Ticks)
$manualSamples = [Collections.Generic.List[object]]::new()
function Save-M0ManualSample {
    param([Parameter(Mandatory = $true)][string]$Phase)
    $activeIds = [Collections.Generic.HashSet[int]]::new()
    foreach ($knownId in @($manualBirths.Keys)) {
        try {
            $known = Get-Process -Id $knownId -ErrorAction Stop
            if ($known.StartTime.ToUniversalTime().Ticks -eq $manualBirths[$knownId]) {
                $activeIds.Add($knownId) | Out-Null
            }
        } catch { }
    }
    $tree = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name)
    do {
        $added = $false
        foreach ($entry in $tree) {
            $entryId = [int]$entry.ProcessId
            if (!$activeIds.Contains([int]$entry.ParentProcessId) -or $activeIds.Contains($entryId)) { continue }
            try {
                $parent = Get-Process -Id $entry.ParentProcessId -ErrorAction Stop
                if ($parent.StartTime.ToUniversalTime().Ticks -ne $manualBirths[[int]$entry.ParentProcessId]) { continue }
                $child = Get-Process -Id $entryId -ErrorAction Stop
                if ($child.StartTime -lt $parent.StartTime) { continue }
                $manualBirths[$entryId] = $child.StartTime.ToUniversalTime().Ticks
                $activeIds.Add($entryId) | Out-Null
                $added = $true
            } catch { }
        }
    } while ($added)
    $rows = @(foreach ($entryId in $activeIds) {
        try {
            $process = Get-Process -Id $entryId -ErrorAction Stop
            if ($process.StartTime.ToUniversalTime().Ticks -ne $manualBirths[$entryId]) { continue }
            [pscustomobject]@{
                id = $entryId; name = $process.ProcessName
                started_at_utc = $process.StartTime.ToUniversalTime().ToString('o')
                working_set_bytes = $process.WorkingSet64; private_bytes = $process.PrivateMemorySize64
                handles = $process.HandleCount
            }
        } catch { [pscustomobject]@{ id = $entryId; unmeasured = $true } }
    })
    $measuredRows = @($rows | Where-Object { $_.unmeasured -ne $true })
    $sample = [pscustomobject]@{
        phase = $Phase; measured_at_utc = [DateTime]::UtcNow.ToString('o')
        root_id = $manualApp.Id; processes = $rows; process_count = $rows.Count
        shell_count = @($measuredRows | Where-Object { $_.name -in @('powershell', 'pwsh', 'wsl') }).Count
        unmeasured_count = $rows.Count - $measuredRows.Count
        working_set_mb = [math]::Round(($measuredRows | Measure-Object working_set_bytes -Sum).Sum / 1MB, 2)
        private_mb = [math]::Round(($measuredRows | Measure-Object private_bytes -Sum).Sum / 1MB, 2)
        handles = ($measuredRows | Measure-Object handles -Sum).Sum
    }
    $manualSamples.Add($sample)
    $json = ConvertTo-Json -InputObject $manualSamples.ToArray() -Depth 6
    [IO.File]::WriteAllText($manualSamplesPath, $json, [Text.UTF8Encoding]::new($false))
    $sample | Select-Object phase, process_count, shell_count, unmeasured_count, working_set_mb, private_mb, handles | Format-Table
}
```

调用示例（按第 6 节实际阶段执行，不要一次把所有名称都运行）：

```powershell
Save-M0ManualSample 'initial-one-tab'
Save-M0ManualSample 'round-1-twenty-tabs'
Save-M0ManualSample 'round-1-one-tab-after-30s'
Save-M0ManualSample 'window-closed-after-30s'
```

`shell_count` 包含已识别的 PowerShell/WSL 进程，不把 conhost 或嵌套 shell 数直接等同于 PTY 会话数；每条样本保留名称和 PID 供复核。工作集之和可能重复计算共享页面，`private_mb` 另列。`unmeasured=true` 表示访问失败或采样竞争，要保留该标记；`unmeasured_count>0` 时汇总资源是不完整的。整窗关闭后为 0 只证明此前被采到的本次进程均已退出；没有采到的短命进程、GPU 上下文、监听器和 Rust 内部任务不在这个统计范围内。
