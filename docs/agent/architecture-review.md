# Architecture Review

日期：2026-10-09  
范围：`docs/PLAN.md`、M0 任务与 B/C 报告、当前 Rust/TypeScript 源码和测试。工作区没有 `.git`，无法对历史提交进行 diff 比较。  
性质：架构评估及整改追踪；原评估结论保留为历史基线，当前状态以下面的整改结果和 board.md 为准。

## 2026-10-09 整改结果

维护者在评估后授权修复。现已完成本轮可落地的核心与架构整改，PLAN 已记录共享终端 crate 和局部 PTY 补丁的决定，里程碑范围不变。

| 原发现 | 当前状态 | 落实位置与证据 |
| --- | --- | --- |
| EOF 输出截断、取消不等待 | `accepted` | `crates/capstan-terminal/src/output.rs` 关闭接收后排空；取消等待 pump 完成。100 次 EOF 回归和满队列取消测试通过。 |
| 字节预算缺失 | `accepted` | admission 时登记字节，待合并与 in-flight 共用预算；单块最多 64 KiB，输出上限为 1 MiB + 一个块。100 MiB 完整 pump 测试覆盖顺序、内容与上限。 |
| Tauri 内会话编排过重 | `accepted` | `TerminalService` 拥有读、写、pump、转发、退出监控及启动 worker；窗口关闭等待启动和全部会话回收。Tauri 只转换和转发。 |
| 前端视图与本地后端绑定 | `accepted` | 控制器、transport、显示组件分开；标签 ID 与后端会话 ID 分开。Zustand 存布局/状态，语言和可见性变更不会重启终端。 |
| 输入失败自动重放 | `accepted` | 不确定的写入失败关闭并清空输入队列，后续按键不重放原请求。 |
| IPC 契约漂移 | `accepted` | 集中 TS 命令/响应/状态类型，Rust 稳定错误代码；二进制帧与状态共用 JSON fixture。ACK/resize 串行化并处理错误。 |
| ConPTY 配置未接入 | `accepted` | 实际 spawn 接收运行时选择；局部 portable-pty 补丁按绝对路径加载，支持 Conpty 前缀和旧导出，失败重建系统会话。真实 PowerShell 测试覆盖无效 DLL 回退、退出输出、resize 与关闭。 |
| 指标仅有空模型 | `accepted`（状态通路） | 移除未使用的 `TerminalMetricEvent`，改用周期性 `TerminalSnapshot`，包含队列、水位峰值、累计字节、pause/resume 和最终退出状态，经独立状态 Channel 发射。 |
| 真实设备与性能基线 | `needs-maintainer-manual` / `blocked` | WebView2、20 标签完整测量、实际 NuGet 包/ARM64、IME/DPI 仍需验收；原生 Snap hit-test 仍未实现。 |

新增共享 crate 不依赖 PTY/SSH/Tauri。当前会话 I/O 适配器服务于阻塞式本地 PTY；接入 russh 时应在共享协议/预算边界接入异步 SSH 通道，SSH 连接池仍由 M1 的 SSH crate 单独拥有。M1/M3 的连接 ID、连接复用、窗格树和重连功能没有提前实现。

`vendor/portable-pty/CAPSTAN_PATCH.md` 记录补丁范围、上游版本、许可、API 依据和维护方式。没有加入或分发 ConPTY 二进制。完整检查和手测清单见 board.md。

以下为整改前的评估与复现记录，旧路径 `capstan-pty/src/output.rs` 已迁入 `capstan-terminal/src/output.rs`。

## 1. 结论

方案方向良好，现有代码适合继续完成 M0，但目前还不能评价为“扩展性优秀、可以直接承载后续所有功能”。值得保留的是领域 crate 与 Tauri 的分离方向、原始二进制数据路径，以及独立于 React 的终端渲染封装。需要提前收紧的是会话所有权、流结束契约、跨语言接口和本地/远程后端的共同边界。

目前 workspace 只有 `capstan-pty` 和 `capstan-app`；`capstan-ssh`、`capstan-vault` 尚未建立。这符合 M0 范围，但也意味着 SSH 复用、认证、Vault 和 SFTP 的可扩展性目前主要体现在方案中，不能当作已有实现的能力。

| 后续工作 | 当前基础 | 扩展成本判断 |
| --- | --- | --- |
| 完成本地终端、字体、窗口体验 | PTY、渲染器和窗口入口已有基本分工 | 可以继续，但先解决流结束的数据完整性问题 |
| M1 SSH 与 Vault | PLAN 的领域划分和安全边界明确，实际服务尚未建立 | 先统一终端会话接口，避免复制本地启动/关闭逻辑 |
| M2 SFTP | PLAN 明确复用 SSH 连接 | M1 必须先把连接生命周期与终端标签生命周期分开 |
| M3 分屏、广播、重连 | 前端目前是一标签一 PowerShell 组件 | 需要分离连接、会话、标签和窗格，不能沿用一一绑定 |
| 后续跨平台 | 核心 crate 不依赖 Tauri | 窗口适配较易隔离；Shell 探测和 PTY 运行时选择仍需明确平台接口 |

## 2. 值得保留的结构

- `capstan-pty` 没有 Tauri 依赖，Shell、WSL、输出合并器能够独立测试。继续保持基础库不依赖桌面外壳。
- `createTerminalSession` 通过 `sendInput`、`ackOutput`、`onResize` 回调与外部协作，并支持替换 terminal/renderer。这个边界已适合复用到 SSH。
- 输出不进入 React state；原始 Channel 帧、按字节 ACK、可见性控制 WebGL 的方向合理。需要补足契约，而不是替换整条路径。
- PLAN 已明确 SSH 复用键、安全策略隔离、重连不恢复旧 shell、不重放输入，以及 Vault 锁定与连接的关系。这些约束有利于 M1–M3 保持一致。

## 3. 优先处理的问题

### P1：正常结束、取消和任务退出还没有形成完整契约

位置：`crates/capstan-pty/src/output.rs` 的 `finish`、`cancel_and_wait` 和 `finish_receiver.changed()`；`src-tauri/src/lib.rs` 的读取循环与 `ManagedTerminal`。

`finish()` 通过独立 watch 信号结束输出，接收端只刷新已经进入 merger 的数据，没有排空 `input_receiver` 内已接受的数据。正常 EOF 与输入队列竞争时会丢失尾部输出。本次在临时探针中使用单线程 Tokio runtime，连续发送 16 块、每块 256 字节，随后立即调用 `finish()`；100 次均正常返回，却都未收到完整的 4096 字节，接收范围为 0–2048 字节。

此外，`cancel_and_wait()` 只设置取消状态，并不等待 pump/reader/channel 任务结束。应用保存 reader handle，却没有统一保存和等待全部关联任务；自然 EOF 与读取错误都调用 `finish()`，前端也没有独立的退出码/错误状态事件。

建议定义并测试三类不同操作：正常结束停止接收并排空已接受输出；主动取消按策略丢弃待处理输出；关闭等待全部受管任务完成。由同一个会话服务拥有 reader、writer、pump、转发任务和状态事件。窗口命令只请求服务关闭。

### P1：有界消息数还不能保证完整的字节预算

位置：`crates/capstan-pty/src/output.rs` 的 `OutputConfig`、`OutputPumpHandle::send` 和 `BackpressureGate`。

输入 channel 限制消息数，但 `send(Vec<u8>)` 不限制单条字节数；gate 在发出帧时登记字节，没有覆盖输入 channel 中等待合并的数据。当前 PTY 调用方使用固定 16 KiB 读取块，并主动调用 `wait_until_readable()`，但这个使用约定没有被公共 API 强制保证。以后换成 SSH 读取器，很容易遗漏门控或传入更大的块。

建议把队列字节预算和生产者等待内聚到共同传输接口中。统计 Rust 待发送字节和已发送未 ACK 字节；后者已包含前端待处理数据，分段观测时避免重复相加。输入侧也需要明确 Rust 服务的容量和关闭策略，不能只依赖前端串行调用。当前 PTY 写入持有会话互斥锁，若写入阻塞，resize/close 会等待同一把锁；终止路径应保有独立的控制能力。

### P1：会话编排集中在 Tauri 命令中

位置：`src-tauri/src/lib.rs` 的 `tauri_channel` 模块。

该模块同时负责 shell 选择、会话注册、进程启动、阻塞读取、背压衔接、转发、取消和回收。未来直接加入 SSH，会使认证、连接复用和重连进一步堆在应用入口；也难以在不启动 Tauri 的情况下测试完整生命周期。

建议保留 Tauri Channel 编码和命令参数转换，把会话生命周期移入不依赖 Tauri 的服务。共同输出合并/ACK 类型目前放在 `capstan-pty` 内；接入第二个终端后端时，应迁移到职责明确的共享终端模块，避免 SSH 依赖本地 PTY crate 或复制一套流控。是否新增 crate 属于待确认的工程结构调整，不在本轮实施。

### P2：前端视图与后端会话绑定过紧

位置：`ui/src/terminal/TerminalPane.tsx`、`ui/src/App.tsx`。

`TerminalPane` 自己探测和选择 PowerShell、启动进程、解析协议、调用 ACK/resize/stop，并在 React effect 中拥有整个会话。effect 依赖 `labels`；未来增加语言切换时，标签对象改变可能触发关闭与重新启动。当前 tab 模型只有一个 `id`，还不能表达 SSH 连接共享和嵌套窗格。

建议将后端连接控制器与显示组件分开，显示组件消费稳定的终端会话接口；Shell/主机选择来自明确的启动配置。M1 区分 `ConnectionId`、`TerminalSessionId` 与 `TabId`，M3 再引入窗格树和 `PaneId`。Zustand 保存布局及可序列化会话状态；xterm 实例和输出字节继续留在独立控制器中。显示隐藏、布局移动和用户主动关闭需要分别定义生命周期。

输入发送失败后，目前队首仍保留，下一次输入会重新发送。接入 SSH 前必须明确“未发送”与“结果不确定”的区别，并给新连接/新会话独立代次，防止把旧输入重放到重连后的会话。

### P2：IPC 缺少可检查的统一契约

位置：`ui/src/terminal/bridge.ts`、`ui/src/terminal/frame.ts`、`src-tauri/src/lib.rs`。

`invokeTauri<T>(command: string, args?: Record<string, unknown>)` 允许调用者自行声明返回类型，TypeScript 无法发现命令名、参数或 Rust 返回结构漂移。错误同时混用稳定代码和自由字符串，多个调用直接忽略错误。二进制编码与解码已有各自测试，但尚无共同测试向量保证两端一致。

建议集中定义命令参数、响应和状态事件；稳定错误代码映射到 i18n，原始错误仅进入经过筛选的诊断。优先补统一类型定义及 Rust/TypeScript 共用的帧测试向量，再按接口规模决定是否生成绑定。凭据仍只在 Rust 侧流转。

### P1：ConPTY 选择不是可执行的运行时边界

位置：`crates/capstan-pty/src/conpty.rs`、`crates/capstan-pty/src/session.rs` 的 `PtySession::spawn`、`src-tauri/src/lib.rs` 的 `runtime_summary`。

`select_conpty()` 返回路径和版本元数据，但 `PtySession::spawn()` 直接使用 `native_pty_system()`，没有消费这份选择；summary 还使用默认空捆绑配置。因此目前无法从 Capstan 的选择记录证明启动会话使用了指定运行时。该项同时存在代码接线缺口和实机核验需求，不能仅标为等待维护者手测。

建议由实际启动适配器返回最终实现信息，把选择、加载、启动失败回退和诊断串成一条可测试路径，再补 Windows 版本/架构/签名与许可核验。

### P2：观测和测试集中在零件层

`TerminalMetricEvent` 尚未接入事件发射，pump 结果在 Tauri 转发任务中被忽略。已有测试有效覆盖了若干独立结构，但没有覆盖正常 EOF 排空、完整会话退出与失败状态传播，因此工程检查全绿仍会遗漏本次数据丢失问题。

建议优先补可替换后端的会话契约测试：排空、取消、满队列、延迟/异常 ACK、输入失败、启动途中关闭、自然退出、两个会话相互隔离。运行时 feature 需要独立检查；真实 Windows/WebView2 测试和性能基线继续按 M0 清单验收。

## 4. 推荐实施顺序

1. **M0 收口：** 修复 EOF 排空，明确取消与等待语义；补完整字节预算和状态/指标通路，接通实际 ConPTY 选择。保留现有 Raw Channel、帧顺序校验与渲染生命周期。
2. **M1 接入前：** 确认共享终端服务的归属；抽出不依赖 Tauri 的生命周期服务和前端控制器，建立类型化 IPC、稳定会话 ID 与错误事件。
3. **M1 实施中：** 在 SSH crate 内建立独立连接池与通道所有权。终端只占有一个 shell 通道；SFTP、状态采集和隧道按各自生命周期占用连接，最后一个通道释放后再执行 PLAN 的延迟断开策略。Vault 通过 Rust 侧编排提供认证和信任能力。
4. **M2–M3：** 在上述所有权模型上加入文件任务、窗格树、广播和重连。旧 shell、旧输入和新连接分代处理，终端回滚与底层连接结束分别管理。

本轮建议保持现有技术路线，以契约和资源所有权为中心整理边界；新增共享 crate 的具体方案须按 AGENTS.md 与维护者确认后再写入 PLAN。

## 5. 整改前验证记录

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test --workspace` | 28 个测试通过：PTY 27、应用 1 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo check -p capstan-app --features tauri-runtime` | 通过 |
| `pnpm -C ui typecheck` | 通过 |
| `pnpm -C ui test` | 4 个文件、11 个测试通过 |
| 独立 EOF 探针 | 100 次中 100 次输出截断；每次已接受 4096 字节，收到 0–2048 字节 |

探针使用当前 crate 的公共 API，在临时 Cargo 工程中离线运行，没有修改核心源码。每次执行的关键步骤为：

```rust
let (handle, mut stream, task) = OutputPump::start(OutputConfig::default());
for _ in 0..16 {
    handle.send(vec![0x61; 256]).await?;
}
handle.finish();
let mut received = 0;
while let Some(frame) = stream.recv().await {
    received += frame.payload.len();
    handle.ack(frame.payload.len()).await?;
}
task.await??;
assert_eq!(received, 4096);
```

该探针是整改前的数据丢失证据。缺陷现已由共享终端 crate 的回归测试覆盖并修复；真实 PTY/WebView2 验收范围仍按当前 board.md 执行。
