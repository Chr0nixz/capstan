# M0-B 实施记录

日期：2026-10-05

结论：**补充验证后进入 M1**。本次提交建立了可以重复运行的本地终端核心和前端生命周期夹具，A 门的核心数据结构、合并、ACK、背压和取消已有自动化覆盖；真实 Tauri/WebView2 通道、ConPTY/PowerShell 交互以及 B 门的窗口、字体、IME、DPI 和 20 标签测量仍需要维护者在 Windows 11 设备上完成。

## 修改内容

- 建立 Cargo workspace，新增不依赖 Tauri 的 `crates/capstan-pty`，并加入 `portable-pty` 会话封装。会话支持固定 shell 参数、工作目录、环境变量、读写、resize、正常退出和强制关闭，错误和状态均有明确类型。
- 实现 PowerShell 5.1/7 候选探测与偏好选择。WSL 适配器优先读取 Windows 注册表，数据缺失或读取失败时使用固定参数的 `wsl.exe --list --verbose` 回退，并限制探测超时；UTF-8/UTF-16 输出解析和回退原因可测试。
- 实现 ConPTY 选择记录：捆绑路径必须为绝对路径、文件存在且架构匹配，否则记录拒绝原因并回退系统实现。当前只记录选择和诊断，不把未经许可、签名和包内容核验的 DLL 放入发布内容。
- 实现输出传输核心：16 ms 或 64 KiB 合并、带序号的 `BinaryFrame`、有界输入/输出 channel、ACK 字节计数、积压严格大于 1 MiB 时暂停、严格低于 256 KiB 时恢复、通道关闭和取消唤醒。指标只包含字节数、队列水位、暂停/恢复次数和错误码，不包含终端正文。
- 新增 100 MiB 带模式数据完整性测试、阈值边界测试、过量 ACK 测试和输出 channel 满载时的取消测试。
- 新增 `src-tauri` 粘合层和可选 Tauri 2 runtime。`Channel<Vec<u8>>` 通过 `TauriBinaryChannel` 直接发送二进制帧，并实现核心 `FrameSink`；默认构建不要求 WebView2，启用 `tauri-runtime` 时可编译最小 `tauri::Builder`。未启用通用 shell/fs 插件。
- 新增 `ui` React/Vite 工程和 xterm.js 终端会话模块：fit/unicode11 插件边界、输入有界队列、按 `write` 完成回调发送实际字节数 ACK、resize 回调、可见时挂载 WebGL、隐藏/销毁时释放 renderer 和监听器。前端文案集中在最小的语言表中。
- 新增 workspace、Tauri 配置、最小权限 capability、前端测试配置和依赖锁文件。未修改 `docs/PLAN.md`。

## 自动化结果

以下命令在 Windows 工作区 `D:\Projects\capstan` 执行：

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test --workspace` | 通过，`capstan-pty` 21 个测试，应用 crate 无测试失败 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo check -p capstan-app --features tauri-runtime` | 通过，Tauri 2.12.1 运行时 feature 可编译 |
| `pnpm -C ui typecheck` | 通过 |
| `pnpm -C ui test` | 通过，2 个测试文件、5 个用例 |
| `pnpm -C ui build` | 通过；产物 JS gzip 前约 617 KiB，Vite 给出拆包提示 |

自动化测试覆盖了合并边界、顺序、二进制 0/255 字节、1 MiB/256 KiB 背压边界、重复关闭、取消、输入队列满载、WebGL 可见性切换和监听器释放。真实 shell、真实 WebView2 和窗口管理器没有由这些测试替代。

## 维护者手测清单

下列项目尚未在真实 Windows 11 设备上由维护者确认，不能标记为 M0 通过：

- [ ] PowerShell 5.1/7 启动、方向键、退格、Ctrl+C、中文/emoji、resize 和退出。
- [ ] 首选 ConPTY、捆绑 DLL 缺失/架构不符/启动失败时的系统回退；记录实际版本、架构和绝对路径。
- [ ] 真实 Tauri Channel 的原始二进制顺序、完整性、100 MiB 输出、延迟 ACK、暂停/恢复和关闭标签。
- [ ] WebView2 中可见/隐藏标签的 WebGL 创建、释放、重新挂载和降级到 canvas。
- [ ] 注册表成功、数据不完整、无 WSL、无默认发行版、指定目录错误和 `wsl.exe` 超时回退。
- [ ] Mica、无边框标题栏拖拽、最小化/最大化/还原/关闭、Snap Layouts hit-test、高对比度和 reduced-motion。
- [ ] Cascadia Mono、中文回退、CJK/emoji/组合字符、IME 候选框、高 DPI 和多显示器移动。
- [ ] 创建、切换、关闭 20 个标签并重复 3 轮，记录工作集、私有字节、句柄、WebGL 上下文、PTY 进程、队列和输入响应。
- [ ] 日志和错误输出不包含终端正文、密码、私钥或其他凭据。

## 已知风险

1. `ConPtySelection` 目前是运行时选择和可观测性层；`portable-pty` 的实际 ConPTY DLL 加载路径、NuGet 版本、签名、架构包和再分发许可尚未完成核验。发布配置已关闭 bundle，不能把当前代码解读为已完成捆绑发布。
2. Tauri Channel 适配器已经通过编译，但尚未在 WebView2 中做 payload 类型和传输错误的端到端 smoke test。当前 `OutputPump` 的自动化输出端是 Rust 有界 channel，实际 PTY 读取循环仍需在应用层调用 `wait_until_readable()` 才会暂停读取。
3. 当前应用入口展示最小终端工作区和生命周期夹具，尚未把真实 `PtySession` 读取任务、输入发送命令和窗口关闭事件接到 Tauri builder；因此不能声称已经完成“启动 PowerShell 并交互”的端到端纵向切片。
4. Mica、Snap Layouts 原生 hit-test、标题栏按钮、IME、DPI、多显示器和字体实际命中仍是手测项；配置中的无边框窗口不等于这些行为已经实现。
5. 100 MiB 自动化测试验证了合并器的数据完整性和序号，但还没有在真实 Channel、前端延迟 ACK、PTY 读取暂停和 20 标签循环下生成性能基线文件。
6. Vitest 使用 jsdom 的 canvas stub 验证生命周期逻辑；WebGL 上下文创建、丢失和恢复必须在 WebView2 中重新验证。

## 未决问题

- 锁定哪一个 ConPTY 运行时版本、x64/ARM64 包布局、签名检查方式和第三方声明文件位置。
- Tauri 2/WebView2 对 `Channel<Vec<u8>>` 的实际二进制编码、最大 in-flight 行为和传输错误语义，需要用固定序号/校验夹具确认。
- 标题栏 Mica 与内部 Graphite 表面的最终 API 方案，以及最大化按钮覆盖原生 hit-test 的 Windows 实现方式。
- 性能基线保存格式、固定测试机器和 20 标签测试的句柄/WebGL 采样方式。
- `PtySession` 读取任务、窗口销毁和前端卸载之间的最终取消顺序；需要在接入 Tauri 命令后补资源回收集成测试。

上述问题若改变 `docs/PLAN.md` 的架构、验收标准或里程碑顺序，应先由维护者确认，再单独更新 PLAN 的决策记录。本次没有修改 PLAN。
