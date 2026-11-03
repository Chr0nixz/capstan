# M1-00 本轮重点手测

日期：2026-10-10。按维护者指示，本轮只检查会影响后续终端/SSH 开发的三项，其余测试暂缓。无需重复自动化矩阵，不用填写完整验收模板或粘贴长篇资源采样代码。

**执行结果：本轮三项已通过。** KEY-01 原始报告五场景复核通过；KEY-02/03 按维护者“剩下全部成功”的反馈记录。实际证据与范围见 [`验收记录`](baselines/M0/raw/2026-10-10-010106-priority-ca5bab70/README.md)。以下步骤保留用于复现，本轮无需再次执行。

预计 10–15 分钟；基准实际耗时随设备变化。当前工作区的两份 exe 已准备好，无需重建。源码或 target 变化后的重建步骤见 [`完整说明附录 A`](M1-00-manual-tests.md#附录-a清理-target-或源码变化后重建两种入口)。

| 优先项 | 为什么影响后续开发 | 本轮范围 |
| --- | --- | --- |
| KEY-01 可见全链路 | SSH 将复用二进制 Channel、ACK、背压和 xterm；真实 WebView2 异常会影响所有终端 | 运行一次自动五场景，观察窗口并保存报告 |
| KEY-02 真实输入/尺寸 | SSH shell 同样使用当前输入、Ctrl+C、IME 与 resize 链路 | 一个默认 PowerShell，检查按键、中文和窗口 resize |
| KEY-03 标签隔离/关闭 | 多个 SSH 通道会复用当前控制器、标签和取消机制 | 5 标签一轮，切换、输出中关闭、快速关闭和整窗重开 |

## 准备

在应用外部打开 PowerShell，粘贴以下代码。此后的仓库命令在这个窗口执行；输入体验测试的命令才输入 Capstan。

```powershell
Set-Location 'D:/Projects/capstan'
$ErrorActionPreference = 'Stop'
$benchExe = 'D:/Projects/capstan/target/m0-manual/benchmark/capstan-app.exe'
$desktopExe = 'D:/Projects/capstan/target/m0-manual/desktop/capstan-app.exe'
$stamp = Get-Date -Format 'yyyy-MM-dd-HHmmss'
$suffix = [Guid]::NewGuid().ToString('N').Substring(0, 8)
$evidenceRoot = "D:/Projects/capstan/docs/agent/baselines/M0/raw/$stamp-priority-$suffix"
New-Item -ItemType Directory -Path $evidenceRoot | Out-Null
Write-Output "Evidence directory: $evidenceRoot"
```

## KEY-01：可见全链路，约 1–3 分钟

在外部 PowerShell 运行一次：

```powershell
$runDir = Join-Path $evidenceRoot 'visible-once'
./scripts/Measure-M0.ps1 -Executable $benchExe -OutputDirectory $runDir -Visible -TimeoutSeconds 900 |
    Tee-Object -FilePath (Join-Path $evidenceRoot 'validation.txt')
$report = Get-Content -LiteralPath (Join-Path $runDir 'terminal-report.json') -Raw | ConvertFrom-Json
if (!$report.window_visible) { throw 'The report does not confirm a visible window.' }
```

窗口出现后置于前台，保持当前显示器/缩放和默认窗口大小；观察即可，不输入、不取消、不提前关闭。程序自动完成 bulk、delayed_ack 和三轮 20 会话，再验证和清理；这三轮操作底层会话，不是普通标签栏。

**通过：** PowerShell 输出 `bulk:0`、`delayed_ack:0`、`tab_lifecycle:1/2/3` 五条 `verified`，没有异常。窗口能显示输出，进度推进，完成后自动关闭。报告目录有三份 JSON：terminal-report、process-samples、environment。其他验收仍需手测的提示正常。

**失败：** 有异常、一直无进度、无可见窗口、缺场景或验证器拒绝，保存错误文本和目录路径交回，我来分析。不要改 JSON 或按名称结束其他应用的进程。

本轮只取一次可见功能证据；不能当成原完整清单要求的三次独立性能基线。吞吐、探针响应和资源数据由我分析，你不用计算。

## KEY-02：真实输入与 resize，约 3–5 分钟

在外部 PowerShell 启动普通版：

```powershell
$manualApp = Start-Process -FilePath $desktopExe -WorkingDirectory (Split-Path $desktopExe) -WindowStyle Normal -PassThru
```

应出现普通标签栏和一个默认 PowerShell 提示符，不能是基准页面。只测本机默认 shell，不安装其他版本、不找不存在的 Shell 选择器。

在 **Capstan 内** 依次操作：

1. 手动输入 `Write-Output 'CAPSTAN_OK'` 并回车。按 ↑ 调历史，用 ←/→ 和 Backspace 修改字符串再回车。预期不丢字、不重复执行，提示符正常返回。
2. 输入下面的尺寸命令，拖窄、拖宽窗口后分别再运行一次。预期列/行随实际终端变化，输入长行时换行与光标位置正确。

```powershell
'cols={0}, rows={1}' -f [Console]::WindowWidth, [Console]::WindowHeight
```

3. 运行下面的持续输出，约 2 秒后按 Ctrl+C，再执行第 1 步。预期输出停止，提示符返回，后续输入仍可用。

```powershell
1..1000 | ForEach-Object { Write-Output ('tick {0:D4}' -f $_); Start-Sleep -Milliseconds 100 }
```

4. 手动输入 `Write-Output '`，切中文输入法输入“中文测试”，观察候选框，再提交并补 `'` 回车。预期候选框靠近光标，中文只提交一次且不乱码；粘贴中文不能替代这个测试。

任一步失败，记录步骤、实际现象和复现次数即可。截图只使用合成测试文字。正常情况下不需要收集终端全文。

## KEY-03：5 标签与关闭，约 3–5 分钟

1. 在普通应用点 `+` 四次，总共 5 标签；分别输出 `'TAB_A'`、`'TAB_B'`、`'TAB_C'`、`'TAB_D'`、`'TAB_E'`。来回切换两遍，预期各标签显示自己的标记，输入落在当前标签，不串输出、不黑屏。
2. 在一个标签运行 KEY-02 的持续输出，切到另一个标签运行 `Write-Output 'STILL_ALIVE'`。预期前台标签仍响应，切回输出标签时能继续看到输出。
3. 直接关闭持续输出的标签，再在剩余标签执行 `Write-Output 'STILL_ALIVE'`。预期剩余标签正常，已关闭标签的内容不出现到其他标签。
4. 点 `+` 后立即关闭新标签，做两次。预期应用可继续新建终端，没有迟到输出串入其他标签；若两次都已出现提示符才关掉，记录“快速关闭正常，未命中启动阶段”。
5. 再开一个持续输出标签，点击右上角关闭整窗。等待约 10 秒，重新执行 KEY-02 的启动命令，确认能再次出现提示符并运行 `Write-Output 'CAPSTAN_OK'`，然后关闭。

**通过：** 切换/关闭不串会话、不重复输入、不挂起，输出中关闭和整窗重开正常。关闭最后一个标签会自动新建一个，这是当前行为；整窗关闭另测。

这是普通标签栏的轻量冒烟检查，不是三轮真实 20 标签的完整验收，也不能单凭窗口消失证明没有后台任务或 GPU 泄漏。资源计数的已有自动化证据保留；发现明显异常再由我定向采样。

## 只需交回这些

- KEY-01 的 `$evidenceRoot` 目录路径，失败时另附错误文本。
- KEY-02 / KEY-03 各写“通过”或“失败：具体步骤和现象”；若某个子步骤没测，明确指出。
- 当前显示缩放、是否远程桌面。其他机器信息已由 KEY-01 脚本采集。

可直接这样回复：

```text
KEY-01: Passed / Failed; evidence directory: ...
KEY-02: Passed / Failed; step and actual behavior: ...
KEY-03: Passed / Failed; step and actual behavior: ...
Scaling: ...; Remote desktop: Yes / No
```

Snap、Mica、DPI/多屏矩阵、读屏/对比度、原生 WSL、有效捆绑 ConPTY、ARM64、GPU 故障注入及完整性能/20 标签基线，按维护者指示本轮暂缓。记录为未测或原工程缺口，保留原验收要求；本次安排不修改 PLAN，不宣称 M0 总体验收完成，也不自动授予进入 M1 的验收结论。
