# M0-C 独立检查报告

日期：2026-10-05  
检查对象：M0 阶段实现与 `docs/agent/M0-B.md` 交付物  
依据：`docs/agent/M0.md`、`docs/PLAN.md`、`AGENTS.md`  
检查重点：错误路径（Error Paths）、边界条件（Boundary Conditions）、资源生命周期与回归问题

---

## 1. 总体评价与工程检查

在当前代码库中，B 阶段建立了 `crates/capstan-pty`、`src-tauri` 与 `ui` 的基本模块骨架，实现了有界输出合并器、背压门控与基础前端 Session 逻辑。静态工程检查通过情况如下：

| 命令 | 检查项 | 结果 |
| --- | --- | --- |
| `cargo fmt --all -- --check` | Rust 代码格式 | 通过（无输出） |
| `cargo test --workspace` | Rust 单元测试 | 通过（21 个用例全部通过，耗时约 0.41s） |
| `cargo clippy --workspace --all-targets -- -D warnings` | Rust 静态分析 | 通过（无警告） |
| `cargo check -p capstan-app --features tauri-runtime` | Tauri 2 运行时 feature 编译 | 通过 |
| `pnpm -C ui typecheck` | 前端 TypeScript 类型检查 | 通过 |
| `pnpm -C ui test` | 前端 Vitest 测试 | 通过（2 个测试文件，5 个用例全部通过） |

**核心结论**：静态检查与原有单元测试全部通过，但深入审查错误处理、异步协同、Windows 原生交互及真实边界条件后，发现多处可能导致**死锁、进程泄漏、数据解析失败、DLL 入口缺失及通道契约断裂**的关键缺陷。当前代码尚未形成可靠的端到端纵向切片。

---

## 2. 重点问题发现与复现记录

### 问题 1：背压暂停状态下 Channel 接收端关闭导致 `wait_until_readable` 永久死锁（错误路径 / 异步取消）

- **涉及文件**：`crates/capstan-pty/src/output.rs:379, 400, 434`
- **问题分析**：
  在 `OutputPump::start` 的循环中，当向下游 `output: mpsc::Sender<BinaryFrame>` 发送数据失败时（例如前端标签关闭、组件卸载、WebView2 异常导致 `OutputStream` 释放），`send_frame` 通过 `?` 运算符直接返回 `Err(PtyError::OutputClosed)` 退出当前后台 task：
  ```rust
  result = output.send(frame) => result.map_err(|_| PtyError::OutputClosed)?
  ```
  此时跳过了循环末尾的清理逻辑：
  `gate.cancel().await` 和 `gate.mark_channel_closed().await` **均未被调用**。
  如果此时 `gate.is_paused()` 为 `true`（即未确认数据已超过 1 MiB 暂停水位），`state.closed` 依然保持 `false`，`state.paused` 依然保持 `true`。任何在 `handle.wait_until_readable()` 上等待恢复读取的 PTY 读取线程或工作任务将**永远休眠、发生永久死锁**，无法被唤醒。

- **复现测试**：
  在 `output.rs` 的测试模块中添加用例：填充超过 1 MiB 触发暂停，随后 `drop(stream)` 关闭接收端，再向 handle 发送一个块触发 `OutputClosed`，然后调用 `handle.wait_until_readable()`：
  ```rust
  #[tokio::test]
  async fn output_stream_dropped_while_paused_hangs_wait_until_readable() {
      let (handle, mut stream, _task) = OutputPump::start(OutputConfig::default());
      for _ in 0..17 {
          handle.send(vec![42u8; MAX_FRAME_SIZE]).await.expect("send");
      }
      for _ in 0..17 {
          let _ = stream.recv().await.expect("recv");
      }
      assert!(handle.is_paused().await);
      drop(stream);
      let _ = handle.send(vec![1u8; MAX_FRAME_SIZE]).await;
      let wait_result = tokio::time::timeout(
          Duration::from_millis(100),
          handle.wait_until_readable(),
      ).await;
      assert!(wait_result.is_err(), "wait_until_readable timed out");
  }
  ```
- **实际结果**：
  `wait_until_readable` 无法以 `Err(PtyError::Cancelled)` 或 `Err(PtyError::OutputClosed)` 退出，超时后依然处于阻塞等待状态。

---

### 问题 2：`decode_wsl_output` 无法处理真实 Windows `wsl.exe` 输出，导致发行版解析完全失效（编码与边界条件）

- **涉及文件**：`crates/capstan-pty/src/wsl.rs:182-218`
- **问题分析**：
  `decode_wsl_output` 中对 UTF-16 的判断硬编码为检查首两个字节是否为 BOM (`0xFF 0xFE`)：
  ```rust
  if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] == 0xfe { ... }
  ```
  在 Windows 11 环境下，通过管道重定向执行 `wsl.exe --list --verbose` 时，其实际输出为**不带 BOM 的 UTF-16LE**（以空白字符 `0x20 0x00 0x20 0x00` 开始）。
  实测命令：`powershell -NoProfile -Command "wsl.exe -l -v | Format-Hex"` 输出首行：
  `20 00 20 00 4E 00 41 00 4D 00 45 00 20 00 20 00 ...`（完全没有 `0xff 0xfe`）。
  因缺失 BOM，`decode_wsl_output` 回退至 `String::from_utf8_lossy(bytes)`，使得每两个字符之间充斥 `\0` 空字符。后续 `parse_wsl_verbose_output` 解析行：
  `line.starts_with('*')` 因首字符为 `\0` 而返回 `false`；提取的名称为 `"\0*\0"`，版本数字解析失败，导致在注册表缺失进行回退探测时产生完全错误的数据或空列表。
  B 阶段的单元测试 `parses_utf16_wsl_verbose_output` 人工在数据前插入了 `vec![0xff, 0xfe]`，从而掩盖了该真实环境问题。

- **复现测试**：
  执行不带 BOM 的真实 UTF-16LE 字节序列：
  ```rust
  #[test]
  fn parses_utf16le_without_bom_wsl_output() {
      let text = "  NAME                   STATE           VERSION\r\n* Ubuntu                 Running         2\r\n";
      let mut output = Vec::new();
      for unit in text.encode_utf16() {
          output.extend_from_slice(&unit.to_le_bytes());
      }
      let distributions = parse_wsl_verbose_output(&output);
      assert_eq!(distributions.len(), 1);
      assert_eq!(distributions[0].name, "Ubuntu");
  }
  ```
- **复现命令**：
  `cargo test --package capstan-pty -- wsl::tests::parses_utf16le_without_bom_wsl_output`
- **实际结果**：
  ```text
  thread 'wsl::tests::parses_utf16le_without_bom_wsl_output' panicked at crates\capstan-pty\src\wsl.rs:336:9:
  assertion `left == right` failed
    left: "\0*\0"
   right: "Ubuntu"
  ```

---

### 问题 3：`PtySession::close(false)` 在交互式会话中直接挂起调用线程（错误路径 / 进程控制）

- **涉及文件**：`crates/capstan-pty/src/session.rs:176-198`
- **问题分析**：
  `close` 实现如下：
  ```rust
  self.state = SessionState::Exiting;
  if force {
      self.child.kill().map_err(|error| PtyError::Pty(error.to_string()))?;
  }
  let status = self.child.wait().map_err(|error| PtyError::Pty(error.to_string()))?;
  ```
  当传入 `force = false` 时，代码跳过 `kill`，直接同步调用 `self.child.wait()`。对于运行交互式 Shell（如 `powershell.exe`）的会话，由于既未向 PTY 写入退出指令（`exit\r\n`），也未关闭输入端写句柄（`self.writer`），Shell 进程正等待用户输入，绝不会自行退出。`self.child.wait()` 内部进入 Windows `WaitForSingleObject(hProcess, INFINITE)`，造成调用线程无限期挂起。

- **复现测试**：
  ```rust
  #[test]
  #[cfg(windows)]
  fn close_false_hangs_on_running_powershell() {
      let detector = crate::ShellDetector::new();
      let shell = detector.choose_powershell(crate::PowerShellPreference::Prefer51).unwrap();
      let config = PtySessionConfig { command: PtyCommand { shell, working_directory: None, environment: Default::default() }, columns: 80, rows: 24 };
      let mut session = PtySession::spawn(&config).unwrap();
      let _ = session.close(false);
  }
  ```
- **复现命令**：
  `cargo test --package capstan-pty -- session::tests::close_false_hangs_on_running_powershell`
- **实际结果**：
  测试超时被终止（运行超过 60 秒挂起，无法返回）。

---

### 问题 4：非 `Running` 状态下 `PtySession::drop` 不终止子进程，造成僵尸进程与句柄泄漏（资源释放）

- **涉及文件**：`crates/capstan-pty/src/session.rs:201-207`
- **问题分析**：
  `PtySession` 的 `Drop` 实现为：
  ```rust
  impl Drop for PtySession {
      fn drop(&mut self) {
          if self.state == SessionState::Running {
              let _ = self.child.kill();
          }
      }
  }
  ```
  如果会话尝试过关闭（例如调用 `close(false)` 导致 `self.state = SessionState::Exiting`），或者状态处于 `Starting`、`Cancelled`、`Error` 等状态时被 drop，判断条件 `self.state == SessionState::Running` 均不成立。在 Windows 下，ConPTY 的 `ClosePseudoConsole` 并不会自动级联终止依附在其上的子进程，导致 `powershell.exe` 进程在后台一直残留，耗尽句柄与系统资源。应为只要 `self.state != SessionState::Exited`，在 drop 时就必须确保 `self.child.kill()`。

---

### 问题 5：`try_wait` 与 `close` 在进程退出后二次调用丢失退出码（边界条件）

- **涉及文件**：`crates/capstan-pty/src/session.rs:155-174, 177-182`
- **问题分析**：
  在 `try_wait` 中，首次检测到子进程退出时，返回 `Some(SessionExit { code: Some(status.exit_code()), state: SessionState::Exited })` 并置 `self.state = SessionState::Exited`。
  但随后的幂等调用中：
  ```rust
  if self.state == SessionState::Exited {
      return Ok(Some(SessionExit { code: None, state: SessionState::Exited }));
  }
  ```
  退出状态码丢失（从 `Some(code)` 变为 `None`）。同理，在子进程已退出后调用 `close()` 同样返回 `code: None`。缺少在 struct 中缓存 `exit_code: Option<u32>` 的逻辑，违反了状态查询的幂等性。

---

### 问题 6：Tauri 二进制通道剥离了帧序号 `sequence`（IPC 与数据契约）

- **涉及文件**：`src-tauri/src/lib.rs:48-51`
- **问题分析**：
  `BinaryFrame` 定义为包含 `sequence: u64` 与 `payload: Vec<u8>`。
  然而在 `TauriBinaryChannel::send` 实现中：
  ```rust
  channel.send(frame.payload).map_err(|error| PtyError::OutputSend(error.to_string()))
  ```
  发送给前端 Tauri Channel 的仅为原始 `frame.payload`，`frame.sequence` 被直接丢弃。
  依据 `docs/agent/M0.md` 场景 A3 的要求：
  > “使用可重复的带序号字节模式，例如每个块含递增序号、长度和校验值... 零丢失、零重复、零乱序”
  前端根本无法收到序号，无法对 IPC 传输的端到端顺序与丢包进行独立校验。

---

### 问题 7：前端 `InputQueue` 入队后立即同步出队，导致缓冲与背压机制失效（前端生命周期）

- **涉及文件**：`ui/src/terminal/session.ts:73-83`
- **问题分析**：
  在 `session.ts` 的 `onData` 监听器中：
  ```typescript
  const result = queue.enqueue(bytes);
  if (result.accepted) {
    const queued = queue.dequeue();
    sendInput?.(queued ?? bytes);
  }
  ```
  每次收到 xterm.js 输入，入队后紧接着**同步出队**并调用 `sendInput`。
  这导致 `InputQueue` 在任何时刻长度都恒为 0，`session.flushInput()` 永远返回空数组。当底层 IPC 或 PTY 写入端发生拥塞时，无法对键盘输入进行有效缓冲，队列退化为仅针对单次按键字节大小的简单门限判断，失去了有界队列削峰填谷的作用。

---

### 问题 8：xterm.js Unicode 11 插件未激活、Fit 插件未自适应、字体回退不符合规范（终端显示与字体）

- **涉及文件**：`ui/src/terminal/session.ts:41-47, 61-67`
- **问题分析**：
  1. `terminal.loadAddon(unicode)`：在 xterm.js 中，加载 `Unicode11Addon` 后必须显式执行 `terminal.unicode.activeVersion = '11'` 才能激活 Unicode 11 宽字符处理，当前代码未设置该属性，终端仍运行在默认的 Unicode 6 规则下，Emoji 与复合 CJK 宽度将计算错误。
  2. `terminal.loadAddon(fit)`：创建了 `FitAddon`，但在组件挂载、`setVisible(true)` 及容器尺寸变化时**从未调用 `fit.fit()`**，导致 xterm 默认停留在 80×24 列行，无法根据窗口自动适应。
  3. `fontFamily` 配置为 `'"Cascadia Mono", "Sarasa Mono SC", Consolas, monospace'`。M0 规范明确规定：*“默认终端字体为 Cascadia Mono；中文回退到 Microsoft YaHei UI”*。Windows 11 默认未安装更纱黑体（Sarasa Mono SC），会导致中文字体回退不可预测。

---

### 问题 9：`WslProbe` 与 `select_conpty` 处于孤立状态，未接入实际会话链路（架构与集成）

- **涉及文件**：`crates/capstan-pty/src/session.rs:73-119`、`src-tauri/src/lib.rs:72-84`
- **问题分析**：
  1. `crates/capstan-pty/src/conpty.rs` 实现了 `select_conpty` 与捆绑路径合法性检查，但 `PtySession::spawn` 内部直接调用 `portable_pty::native_pty_system()`，根本没有接收 `ConPtyConfig` 或 `ConPtySelection` 参数，无法利用该选择逻辑加载或回退。
  2. `WslProbe` 的注册表与 fallback 探测逻辑虽然已编写，但 `ShellDetector::new()` 中 `wsl` 列表硬编码为空 `Vec::new()`，`runtime_summary()` 亦未调用 `WslProbe`。
  实测命令：`cargo run -p capstan-app`，即使本机已安装 WSL Ubuntu，控制台依然输出 `detected 1 local shell(s)`（仅探测出 PowerShell 5.1），WSL 探测完全被旁路。

---

### 问题 10：`OutputPump` 刷新定时器未对齐推流时间点，可能产生双倍延迟（输出合并）

- **涉及文件**：`crates/capstan-pty/src/output.rs:370, 377-385`
- **问题分析**：
  `OutputPump` 使用固定周期的 `tokio::time::interval(FLUSH_INTERVAL)`（16ms）。如果数据在间隔 15ms 时到达 `OutputMerger`，下一个 tick 在 1ms 后到达，由于 `flush_due` 判断 `elapsed < 16ms` 返回 `None`，该帧数据必须等待再下一个 tick 才能输出，实际输出延迟达到了 1ms + 16ms = 17ms ~ 31ms。规范要求的“达到 16 ms 或 64 KiB 任一条件即发送”在低频小数据包场景下会因定时器相位差导致轻微抖动。

---

### 问题 11：`src-tauri/build.rs` 条件编译限制导致启用 `tauri-runtime` 启动即崩溃 `STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139)`（构建与运行时）

- **涉及文件**：`src-tauri/build.rs:1-7`
- **问题分析**：
  `src-tauri/build.rs` 的构建脚本如下：
  ```rust
  fn main() {
      if std::env::var_os("CARGO_FEATURE_TAURI_RUNTIME").is_some()
          && std::env::var_os("CAPSTAN_ENABLE_TAURI_BUNDLE").is_some()
      {
          tauri_build::build();
      }
  }
  ```
  在 Windows 下，Tauri 依赖 `tauri_build::build()` 来复制并链接 `WebView2Loader.dll`。由于判断条件绑定了 `CAPSTAN_ENABLE_TAURI_BUNDLE` 环境变量，当执行日常开发或运行命令 `cargo run -p capstan-app --features tauri-runtime` 时，`tauri_build::build()` 完全被跳过，生成的二进制程序无法在运行时加载 WebView2 Loader，直接报入口点未找到崩溃。
- **复现命令**：
  `cargo run -p capstan-app --features tauri-runtime`
- **实际结果**：
  ```text
  Running `target\debug\capstan-app.exe`
  error: process didn't exit successfully: `target\debug\capstan-app.exe` (exit code: 0xc0000139, STATUS_ENTRYPOINT_NOT_FOUND)
  ```

---

### 问题 12：`probe_registry` 任一子键字段缺失导致整体 WSL 探测全盘崩溃（错误隔离）

- **涉及文件**：`crates/capstan-pty/src/wsl.rs:231-253`
- **问题分析**：
  在 `probe_registry` 中遍历 `Software\Microsoft\Windows\CurrentVersion\Lxss` 注册表项时：
  ```rust
  let distribution_name: String = key
      .get_value("DistributionName")
      .map_err(|error| PtyError::WslProbe(format!("missing DistributionName: {error}")))?;
  ```
  若注册表存在非发行版项、已卸载残留项或损坏键值（例如用户删除或第三方软件写入），上述代码直接通过 `?` 抛出错误终止整个枚举过程。
  M0 阶段 4 明确要求：*“实现注册表枚举适配器，隔离 Windows API 读取错误和字段解析错误”*。单个子键缺失字段应被忽略跳过，而非中断整个探测流程。

---

### 问题 13：`ShellDetector` 对 PATH 中的 Shell 返回相对路径而非绝对文件路径（安全与路径规范）

- **涉及文件**：`crates/capstan-pty/src/shell.rs:102-107, 168-189`
- **问题分析**：
  在 `detect_powershells` 中，对 PowerShell 7 的候选路径包含 `PathBuf::from("pwsh.exe")`。`command_on_path` 遍历 PATH 找到文件后返回 `true`，但 `first_existing` 返回的却是原始候选对象 `PathBuf::from("pwsh.exe")`（相对名称），丢弃了在 PATH 中定位到的绝对目录路径。
  这不仅违反了 M0 A1 要求的“列出可执行文件路径”，而且将相对路径直接传给 `CommandBuilder`，在 Windows 复杂工作目录环境下存在由于当前工作目录优先级引发的代码执行安全风险。

---

### 问题 14：PowerShell 5.1 未配置 UTF-8 输出代码页，导致终端输出乱码（字符编码与 CJK）

- **涉及文件**：`crates/capstan-pty/src/shell.rs:40-48`、`crates/capstan-pty/src/session.rs:85-99`
- **问题分析**：
  `ShellInfo::powershell_51` 仅传递参数 `["-NoLogo", "-NoProfile"]`，未配置控制台代码页或环境变量。在中文 Windows 环境下，PowerShell 5.1 默认代码页为 CP936 (GBK)，其输出到 PTY 的中文字符为双字节 GBK 编码。
  然而前端 xterm.js 默认以 UTF-8 解码字节流（`TextDecoder`），非 ASCII 字符与中文将直接呈现为乱码或 Unicode 替换符 ``，严重影响 A2 场景“包含 ASCII、CJK、中文字符”的验收。启动参数或初始化命令需确保代码页切换为 UTF-8（`65001`）。

---

### 问题 15：无边框窗口缺少拖拽区域、系统窗口控制按钮与 Snap Layouts Hit-Test（Windows 集成）

- **涉及文件**：`ui/src/App.tsx:38-46`、`ui/src/styles.css:33-41`、`src-tauri/tauri.conf.json:16`
- **问题分析**：
  在 `tauri.conf.json` 中配置了 `"decorations": false`，但前端自绘标题栏 `<header className="titlebar">` 中：
  1. 未声明 `data-tauri-drag-region` 属性，导致窗口无法通过鼠标按住标题栏拖拽移动。
  2. 完全缺失最小化、最大化/还原、关闭等系统控制按钮。
  3. 未在原生 Rust/Win32 窗口过程或插件中拦截 `WM_NCHITTEST` 返回 `HTMAXBUTTON`，导致 Windows 11 的 Snap Layouts 悬停分屏交互无法触发。

---

### 问题 16：`TerminalMetricEvent` 模型已定义但全链路未埋点发射（可观测性）

- **涉及文件**：`crates/capstan-pty/src/metrics.rs:18-42`
- **问题分析**：
  `TerminalMetricEvent` 与 `TerminalEventKind` 在 `metrics.rs` 中定义完整，但在 `output.rs`、`session.rs` 及 `src-tauri` 中完全没有任何实例化与发射调用。`OutputPump` 内部虽然维护了部分累加指标（`OutputMetrics`），但在运行期间外部无法获知水位序列与瞬时状态，也未通过事件向前端或日志管道输出性能数据。

---

### 问题 17：同步阻塞的 PTY 读取未与 Tokio 异步运行时解耦（并发与架构缺陷）

- **涉及文件**：`crates/capstan-pty/src/session.rs:125-130`
- **问题分析**：
  `PtySession::read` 底层是 `std::io::Read` 的阻塞式系统调用（Windows 命名管道 `ReadFile`）。当前应用层中**没有任何代码启动后台任务连接 `PtySession::read` 与 `OutputPump`**。
  若直接在 Tokio 工作线程中循环调用 `session.read()`，会阻塞 Tokio worker 线程；若采用 `spawn_blocking`，在 Windows 命名管道阻塞读取时，由于缺乏针对管道的取消机制，在会话销毁时读取线程将长时间卡死在 Windows 内核调用中，无法优雅终止。

---

## 3. M0 验收门（A 门 / B 门）完成度对照

对照 `docs/agent/M0.md` 第 3 节中关于 M0 的双门要求进行逐项核实：

### 3.1 A 门：本地终端与 IPC

| 验收场景 | 状态 | 独立检查意见 |
| --- | --- | --- |
| **A1: 启动和探测** | **部分满足** | PowerShell 5.1 探测可用；PowerShell 7 命中 PATH 时返回相对路径；ConPTY 运行时选择未实际接入 PTY 会话；WSL 探测被旁路。 |
| **A2: 输入、输出与退出** | **未通过** | 真实 PTY 的读操作为同步阻塞，未接入 Tokio 异步环境；`close(false)` 会引发无限死锁；非 `Running` 状态下 drop 会泄漏进程；代码页未配置为 UTF-8。 |
| **A3: 二进制通道与背压** | **未通过** | 核心背压测试用例在正常路径下通过；但在 `OutputClosed` 错误路径下存在永久等待死锁；Tauri Channel 丢弃了序号 `sequence`；前端输入队列被同步清空。 |
| **A4: WebGL 生命周期** | **部分满足** | 仅通过 Vitest + jsdom mock 验证了可见性切换时 dispose/mount 回调的调用，真实 WebView2 下的 WebGL 上下文创建、丢失与恢复未进行验证。 |

### 3.2 B 门：Windows 集成与压力

| 验收场景 | 状态 | 独立检查意见 |
| --- | --- | --- |
| **B1: WSL** | **未通过** | 注册表探测已实现但缺少单项容错；受控 `wsl.exe` 回退解析因 UTF-16LE 无 BOM 缺陷完全失效；且整个 WSL 探测未接入应用层。 |
| **B2: 原生窗口** | **未实现** | 仅在 `tauri.conf.json` 中配置了 `decorations: false`；标题栏无拖拽区域、无系统控制按钮、无 Snap Layouts 原生 hit-test。 |
| **B3: 字体与显示** | **未满足** | Cascadia Mono 回退字样配置错误；Unicode 11 与 Fit 插件未正常激活；IME 候选框与多 DPI 未在真实窗口中验证。 |
| **B4: 20 标签压力** | **未执行** | 因前后端端到端粘合层（Tauri 命令与事件闭环）尚未连通，且 `capstan-app.exe` 存在运行时缺少 Loader 的问题，无法执行 20 标签生命周期循环压测。 |

---

## 4. 结论与后续行动建议

### 结论
**维持当前处于 M0 阶段，暂不满足进入 M1 的条件**。
当前代码库具备了良好的模块分层和数据结构定义，但在走向可运行的纵向切片前，必须先修复上述错误路径和集成缺陷。

### 建议修复路径（不改动既定架构）

1. **修复 `OutputPump` 错误路径与死锁**：
   在 `OutputPump` 的循环中，若 `send_frame` 返回 `Err`，在退出循环前显式调用 `gate.cancel().await`，唤醒所有阻塞在 `wait_until_readable` 上的任务并标记通道关闭。
2. **修复 `decode_wsl_output` 编码探测**：
   支持无 BOM 的 UTF-16LE 探测（例如检查偶数索引有效字符与奇数索引 `\0`，或根据文本特征进行宽松解码）。
3. **修复 `PtySession::close` 与 `Drop` 行为**：
   - `close(false)` 增加对 Shell 的优雅退出机制（或在未退出时提供超时机制回退到 kill）。
   - `Drop` 改为检查 `self.state != SessionState::Exited`，统一执行 `child.kill()`，避免进程泄露。
   - 内部缓存 `exit_code: Option<u32>`，确保 `try_wait` 与 `close` 的幂等性。
4. **规范二进制通道帧格式**：
   Tauri Channel 传输的二进制数据应带有固定长度的 Frame Header（包含 `sequence: u64` 与 payload 长度），避免前端接收端丢失序号。
5. **纠正前端 Session 插件配置**：
   激活 `Unicode11Addon`（`terminal.unicode.activeVersion = '11'`），修复默认字体回退到 `"Microsoft YaHei UI"`，并在可见与 resize 时触发 `fit.fit()`。
6. **修复构建与运行时依赖**：
   调整 `src-tauri/build.rs`，确保在 `tauri-runtime` 启用时正确执行 `tauri_build::build()`，从而自动打包复制 `WebView2Loader.dll`。
7. **打通 Tauri Command 粘合层**：
   在 `src-tauri` 补齐启动会话、输入管道、Channel 传递与 ACK 的最小调用接口，使维护者能够在真实 Windows 11 上执行手测验收。
