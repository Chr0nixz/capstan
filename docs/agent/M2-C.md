# M2 独立审查与错误路径/边界条件报告 (M2-C)

<!-- m2-front-current -->
> 2026-10-26 高 RTT 续测：r5 仍为 510 输入；18/18 工程检查、独立代码复核、7/7 非前台定量和工程安装/升级验证已完成，维护者已报告六组桌面测试完成。低 RTT 前台完整通过；高 RTT 已生成三轮报告的焦点均 100%，正式第 2 轮因 SSH ACK 超时中止，完整矩阵未完成，原失败尝试均保留，源码及夹具清理校验通过。M1 焦点 100%，吞吐和本地/混合输入延迟三项仍未达冻结门槛；M2 blocked，v0.2 候选待签收，A 门保留。见[本轮收尾](M2-FOLLOWUP.md)。下方旧检查点保留原范围。
<!-- /m2-front-current -->

> **实施者定量记录，不是独立复核：** 2026-10-22 定量执行收尾：当前 495 输入源码的六组正式文件/目录矩阵均有完整记录，每组含预热和三轮双向、双客户端共 16 个样本；六例 1 GiB 真实 TCP 中断续传通过。M1 与低/高 RTT 可见混合负载、焦点、完整性、资源和清理均通过。性能失败仍保留：tree-high-final-r1: upload data=0.601, duration=1.906 | files-high-final-r2: upload data=0.693, duration=1.159; download data=0.805, duration=1.257。见[完整定量结果](baselines/M2/raw/2026-10-21-quantitative/QUANTITATIVE-RESULTS.md)。独立复核第 7 节的恢复/导出问题、新增桌面验收和 v0.2 候选仍开放；M2 未签收，PLAN 未改。

> **2026-10-21 新增独立复核（不是实施者自查，详见文末第 7 节）：** 对当前实现新增 1 项高优先级恢复记录缺陷和 2 项低优先级边界/交互缺陷。高优先级问题已在进程内 SFTP 夹具复现：清理失败且任务不可续传时，`cleanup_required=true`、远端/本地部分文件仍在，但恢复记录被删除，界面没有清理入口；上传与下载两条路径均可触发。另复现并发导出上限重检失败遗留孤立暂存目录，以及 SFTP 面板在导出授权过期后仍显示可拖拽按钮。基线 329/14/351 项 Rust 测试、30 项原生测试、34 文件/195 项 UI 测试和工程检查均通过，但未覆盖上述路径。既有历史结论与 A 门签收范围保留；B 门/M2 仍为 `blocked`，本轮不构成验收。

> **正式文件矩阵收尾（实施者自查）：** 当前源码 3 × 1 GiB localhost 的预热及三轮测量共 16 个样本通过完整哈希、数据/总耗时/绝对门槛，吞吐比 1.538/1.564、总耗时比 0.583/0.632；最终任务/额度/池和夹具回收通过，进程退出码 0。首个中断尝试无完整样本，原记录和本地删除自动审批拒绝保留；不将中断补写为通过。工程检查、包身份及[交付索引](baselines/M2/raw/2026-10-21-preflight/delivery.json) 可复核。这不关闭高 RTT、默认混合停滞、前台/桌面、独立审查和候选门。

> **本次证据收尾（实施者自查）：** 当前有界预检源码的正式 localhost 目录树 16 个完整样本已复算通过，每样本 128 文件完整哈希、13 目录和最终 payload/jobs/pool 回收一致，上传/下载总耗时比为 0.405/0.328。高 RTT 仍只有单轮诊断，原始路径混合停滞及其余门槛未关闭；新增源码、失败记录和当前包身份独立绑定。见[复算记录](baselines/M2/raw/2026-10-21-preflight/workflow-evidence-audit.json)。本段不代替独立审查或维护者新增桌面验收。

> **有界预检续作（实施者自查，不是独立审查）：** 独立目标快照改为每批至多 3 项；目录树按深度先查祖先，冲突/非法目录下不发起后代 I/O，保留原条目及冲突顺序。乱序、目标绑定和失败阻止下一批的 3 项新回归在旧实现失败、修改后通过；另有原生快照错误时等待既有操作完成、无第四项启动的回归。新一轮 18 项必需检查和 15 项工具检查通过（Rust 329/14/351、UI 195），全局提交锁/完整哈希/预算未变。当前审查包为 `m2-review-873a32487d0e`，0.1.0，见[检查与源身份](baselines/M2/raw/2026-10-21-preflight/verification.json)。真实 SFTP/core、高 RTT 128 文件/13 目录哈希及清理通过，但单轮上传/下载比为 1.950/1.400，上传仍超 1.5；不能把单轮、旧源码性能或实施者自查扩充为新实现的正式验收。

> **同日 SFTP 帧续作（实施者自查）：** 高 RTT 目录树预热暴露小请求开销，已保留八个参考样本和两个产品预热，并通过所属取消请求终止未达标尝试。适配器此前把完整帧的长度和正文分两次发送，新增回归在旧实现失败；现合并为一次 `write_all`，单字节容量的部分写入/背压回归通过。全局提交锁、确认/版本检查、完整哈希与预算不变。新一轮 18 项检查通过（Rust 325/14/347，UI 195，显式外部 skip 单列）；真实性能效果仍需对照，不能将旧源码的 loopback 通过记录写成新实现的最终矩阵。见 [M2-B](M2-B.md) 和[本轮检查](baselines/M2/raw/2026-10-21-closeout/wire-verification.json)。

> **后续实际结果：** 当前 wire 的 SFTP/core 通过，默认 localhost headless 仍失败；高 RTT 目录树完整烟测哈希与清理通过，但上传/下载总耗时比为 2.870/1.317，上传仍超冻结 1.5 门槛。帧合并未证明解决性能差距，不签收完整矩阵。目录树烟测额外逐文件 oracle 导致旧 180 秒工具截止不足，现仅把该诊断进程上限设为 600 秒；正式矩阵仍 2400 秒、冻结性能门槛和协议时限未变。最新 15 项工具检查和源文件身份见 [final verification](baselines/M2/raw/2026-10-21-closeout/final-verification.json)。这仍是实施者自查与测试结果，不是新增独立审查。

> **2026-10-21 工具收尾（实施者自查，不是独立审查）：** 发现 Linux 清理器仅凭生成账号 UID 认领进程会误覆盖容器复用 UID，现改为夹具可执行文件/标题/命令身份及已核验后代，保留 pidfd 和 PID/启动时间复核；13 项回归与真实轮换/失败清理通过。单文件测量独立的进程停止路径可能在退出竞态中掩盖超时，现复用保留句柄的进程树管理并记录独立清理结果，13 项工具检查通过。原始 localhost 混合停滞仍复现，显式路径 headless 进展不签收前台性能；中断的固定矩阵不补写成功。最新结果、清理限制与未决门见 [M2-B](M2-B.md) 和[收尾证据](baselines/M2/raw/2026-10-21-closeout/verification.json)。

> **本轮 B 工程收尾同步（实施者自查，非独立 C 审查）：** 新会话下载按原路径/原属性重新授权，恢复记录绑定原认证策略摘要，旧记录明确标记“需人工检查”并隐藏不可执行动作；真实目录树暴露的 Windows 原生提交结束符问题已有修复和回归。双向 1 GiB、20%/50%/85% 的六次真实 TCP 断线续传均完成完整哈希、partial 清理和额度回收；另有三任务、目录树和反向 shell 背压回归。检查、失败尝试与构建身份见[本轮索引](baselines/M2/raw/2026-10-20-b-engineering/README.md)。完整性能/可见混合负载和 M1 回归、受影响桌面手测、v0.2 候选及独立审查仍未签收，B/M2 保持 blocked。

> **B 续作同步（2026-10-20，实施者自查，非新的独立审查）：** 本轮补齐生产 Explorer 拖入/ExportToken 暂存拖出、目录/名称/类型预检决策、下载残留重新授权、正常请求取消隔离与额度收敛、Bash/Zsh 安装事务及有界递归目录删除。收尾自查另修复重试跨主机/认证策略绑定、旧连接释放、队列终态恢复、恢复列表重复、前缀不匹配保留，以及删除与传输/设置串行等待可能累计 60 秒的退出问题；现在三者并发排空，失败恢复准入。实现、测试、已知风险和审查包见 [M2-B](M2-B.md) 及[工程证据](baselines/M2/raw/2026-10-20-engineering-completion/README.md)。完整固定性能、实际 Windows/编辑器/shell 交互验收、v0.2 安装/便携/升级候选和新增独立审查仍待完成；B/M2 维持 blocked。下方原始独立发现与既有签收范围保留。

> **2026-10-18 最新状态同步：** 原九项修复和维护者“测试全部通过”的既有 [签收范围](baselines/M2/raw/2026-10-18-maintainer/README.md) 保留。后续 [A1 工程证据](baselines/M2/raw/2026-10-18-a1/README.md) 补齐固定低/高 RTT 参考、预算冻结与输入契约，A1–A6 和 A 门 accepted；B 门和 M2 仍 blocked。当前结果和风险见 [M2-B](M2-B.md)。这是实施者自查/工程与维护者反馈的状态同步，不是新的独立 C 审查；下方“传输/本地 grant 仍缺”、A 门未完成和 `needs-maintainer-manual` 等文字属于原审查时点，不作为当前实现/手测结论。

> **2026-10-17 集成修复复核更新（不是新的独立 C 审查）：** 下文原始九项发现均已修复，并由六个新增进程内 pool/service 场景及前端回归验证，修复子范围为 `accepted`。当前 Rust 默认/runtime/全特性报告 162/14/176 项、UI 108 项通过，全部工程检查退出码 0；环境条件跳过和证据范围见 [M2-REVIEW](M2-REVIEW.md)。基础浏览的 Windows 可见交互标记 `needs-maintainer-manual`。单文件传输、本地 grant、后续工作流及关键探针/性能证据仍缺，**M2 A 门、B 门和总体为 `blocked`**；[board](board.md) 是最新签收状态。下文原始代码摘录、行号、97 项 UI 等数字及 `in_progress` 结论仅保留为修复前历史，不能作为当前缺陷或当前验收依据。

> 日期：2026-10-17  
> 审查角色：C（独立审查与验证）  
> 依据：[`M2.md`](M2.md)、[`M2-B.md`](M2-B.md)、[`M2-CONTRACTS.md`](M2-CONTRACTS.md)、[`M2-SFTP-COMPAT.md`](M2-SFTP-COMPAT.md)、[`PLAN.md`](../PLAN.md) 及当前源码  
> 审查范围：实现完整性、错误路径（Error Paths）、边界条件（Boundary Conditions）、安全与作用域隔离、回归问题（Regressions）及契约符合度  
> 验收状态结论：**A 门与 B 门均未达到验收标准（`not accepted`）**，M2 总体状态维持 **`in_progress`**。未产生 v0.2 候选包，未修改 `docs/PLAN.md`，未执行提交或推送。

---

## 1. 审查概览与执行结论

本报告对 M2 阶段已提交的代码与报告进行了两轮独立的实现复核、错误路径分析、权限边界与攻击面测试、全量自动化回归验证。

### 1.1 总体结论

1. **基本分层与架构方向符合规范**：
   - 依赖 `russh-sftp 3.0.1` 仅在 `capstan-ssh` 中作为运行时客户端引入，许可证（Apache-2.0）合规；
   - `capstan-ssh::sftp::SftpWire` 实现了受限的帧长检查（256 KiB 上限）、双向中继与句柄十六进制编解码，防止不透明句柄在第三方库的 UTF-8 转换中损坏；
   - `capstan-core` 中定义了框架无关的 `RemotePath`、`RemoteName`、`Metadata`、`ExactU64`、`ScopeRegistry` 与 `SftpService`，没有引入 Tauri 依赖；
   - `src-tauri` 的 `sftp_commands.rs` 仅做参数转发与窗口校验，未开启通用 shell/fs 插件；文件内容与凭据未进入 WebView。
2. **基线自动化测试无回归**：
   - 全量测试与工程检查均退出码 0（Rust workspace 156 项、runtime 14 项、all-features 170 项、UI 97 项、原生拖拽探针 2 项全部通过，Clippy 零告警，格式检查通过，CSP 与安全检查 10/10，布局检查 28/28 通过）。M0/M1 终端与 Vault 未出现退化。
3. **发现 9 项边界条件、错误路径、安全隔离与交互缺陷**（详见第 3 节）：
   - **缺陷 1（严重边界条件与交互缺陷）**：空目录单次 `readdir` 判定缺陷导致空目录返回 `has_more: true` 并在界面上同时显示“目录为空”与“下一页”按钮。
   - **缺陷 2（严重边界条件/功能阻断）**：双击无法进入指向目录的符号链接（UI 层硬编码过滤 `kind !== "directory"`），直接违反 M2.md §4.4。
   - **缺陷 3（严重错误路径/状态不一致）**：目录导航失败（如路径不存在或无权限）在验证前已调用 `begin_directory` 清空了缓存条目，导致界面上依然可见的旧目录条目后续全部操作均报错 `scope_expired`。
   - **缺陷 4（体验与诊断误导）**：远端不存在路径（`StatusCode::NoSuchFile` / `ClientError::NotFound`）被粗暴映射为 `SftpError::RemoteIo`，前端显示警示性文案“远程操作失败或结果不确定，请刷新核实后再重试修改”。
   - **缺陷 5（错误路径/无效确认流程）**：对符号链接执行 chmod 权限修改时，`prepare_remote_mutation` 未拦截，正常弹出用户确认弹窗；用户点击确认后，`apply_remote_mutation` 才报错 `unsupported_file_type` 失败。
   - **缺陷 6（健壮性/边界阻断）**：目录下存在任一非 UTF-8 或包含替代符的文件名时，`RemoteName::from_library_string(&name)?` 使用 `?` 提前终止，导致整个目录列表完全无法加载浏览。
   - **缺陷 7（安全隔离/作用域逃逸缺陷）**：`create_remote_entry` 缺少当前浏览目录的作用域校验，允许 WebView 提交任意远端父路径在不可见目录创建条目。
   - **缺陷 8（错误路径/死令牌重提）**：应用确认失败时后端单次令牌已被消费，前端弹窗保持打开且未禁用确认按钮，用户再次点击确认被报为毫无上下文的 `scope_expired`。
   - **缺陷 9（交互与分页导航缺陷）**：多页浏览直接替换当前页且立即释放前页条目，界面未提供“上一页”按钮，用户翻页后无法返回前页。
4. **两道验收门达成情况与 B 报告评估**：
   - B 报告（`M2-B.md`）对当前交付范围与验收状态的描述客观，准确指出了 A4 单文件传输、B2-B8 工作流尚未实现。
   - 独立审查确认：**A1～A6 尚未形成闭环，B1～B8 大量未开始，绝不能宣布 A 门或 B 门验收通过**。

---

## 2. 工程检查与测试执行结果

在 Windows x64（Rust 1.96.0，Node 22.23.2，pnpm 10.28.1）环境下执行全量验证，实际结果如下：

| 检查项目 | 执行命令 | 实际结果 | 结论 |
| --- | --- | --- | --- |
| 代码格式 | `cargo fmt --all -- --check` | 退出码 0，无格式差异 | 通过 |
| 默认工作区测试 | `cargo test --workspace` | 156 项测试全部 passed，0 failed | 通过 |
| Tauri Runtime 测试 | `cargo test -p capstan-app --features tauri-runtime` | 14 项测试全部 passed，0 failed | 通过 |
| 全特性测试 | `cargo test --workspace --all-features` | 170 项测试全部 passed，0 failed | 通过 |
| 默认 Clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0，零告警 | 通过 |
| 全特性 Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 退出码 0，零告警 | 通过 |
| Tauri Runtime Check | `cargo check -p capstan-app --features tauri-runtime` | 退出码 0，编译正常 | 通过 |
| 前端类型检查 | `pnpm -C ui typecheck` | 退出码 0，零类型错误 | 通过 |
| 前端单元测试 | `pnpm -C ui test` | 17 个文件，97 项测试全部 passed | 通过 |
| 前端生产构建 | `pnpm -C ui build` | 构建通过（单 chunk 739.97 kB 提示体积警告） | 通过（有体积提示） |
| 前端布局检查 | `pnpm -C ui test:layout` | 缩放 1.0 (14/14) 与 缩放 1.25 (14/14) 全部 passed | 通过 |
| 前端安全检查 | `pnpm -C ui test:security` | 10/10 项安全检查全部 passed | 通过 |
| 原生拖拽探针 | `cargo test --manifest-path vendor/capstan-native-windows/Cargo.toml` | 2 项测试全部 passed | 通过 |
| 依赖与许可证审计 | `cargo deny check all` 与 `Check-Licenses.ps1` | `advisories/bans/licenses/sources` 均 ok；394 Cargo 包与 249 前端包合规 | 通过（有重复版本提示） |

---

## 3. 详细缺陷与边界问题清单

### 缺陷 1：空目录单次 `readdir` 判定缺陷导致返回 `has_more: true` 并在界面同时显示“目录为空”与“下一页”

- **缺陷类别**：边界条件 / 协议交互 / UI 表现缺陷
- **代码位置**：`crates/capstan-core/src/sftp/service.rs` 中的 `page()` (lines 322–345) 及 `ui/src/sftp/SftpPanel.tsx` (lines 161, 173)
- **机理分析**：
  在 `service.rs` 中：
  ```rust
  if cursor.pending.is_empty() && !cursor.ended {
      let names = cursor
          .directory
          .as_mut()
          .ok_or(SftpError::ScopeExpired)?
          .next(&CancellationToken::new())
          .await
          .map_err(map_error)?;
      cursor.ended = names.is_empty();
      for (name, attributes) in names {
          if name == "." || name == ".." {
              continue;
          }
          cursor.pending.push_back((
              RemoteName::from_library_string(&name)?,
              metadata(attributes),
          ));
      }
  }
  let entries = cursor
      .pending
      .drain(..cursor.pending.len().min(super::DIRECTORY_PAGE_ENTRIES))
      .collect();
  let has_more = !cursor.ended || !cursor.pending.is_empty();
  ```
  1. 在标准 Linux/POSIX OpenSSH sftp-server 中，空目录的第一个 `readdir` 包必定返回两个条目：`"."` 和 `".."`。
  2. 此时 `names.len() == 2`，故 `cursor.ended = names.is_empty()` 计算为 `false`。
  3. 循环中由于过滤掉了 `"."` 和 `".."`，`cursor.pending` 仍然为空（`len() == 0`）。
  4. 关键问题在于：代码只使用了 `if cursor.pending.is_empty() && !cursor.ended` 单次读取，而**没有使用循环**继续读取下一个包以检查是否到达 EOF。
  5. 因此，`has_more = !cursor.ended || !cursor.pending.is_empty()` 计算为 `!false || false` = **`true`**！
  6. 结果后端提交并返回了 `DirectoryPage { entries: [], has_more: true }`。
  7. 在前端 `SftpPanel.tsx` 中，`entries.length === 0` 触发渲染 `<p className="sftp-empty">目录为空，可新建文件或目录。</p>`，同时由于 `state.page?.hasMore` 为 `true`，底部又渲染出了 `<button className="sftp-next">下一页</button>`！
  8. 用户面对一个空目录，界面既提示为空又要求翻页；点击“下一页”后才发起第二次网络读取触发真正的 EOF，返回 `entries: [], has_more: false`，下一页按钮才消失。
  9. 同理，若非空目录的第一包只有 `.` 和 `..`，该目录第一页会呈现为“空目录”，导致真实文件被隐藏，必须手动翻页才能看到文件。
- **复现步骤与代码逻辑验证**：
  模拟 SFTP 服务端响应首包 `[(".", attrs), ("..", attrs)]`，第二包 EOF：
  - 观察首页返回：`DirectoryPage { page_sequence: 0, entries: [], has_more: true }`。
  - 前端界面渲染：同时出现 `.sftp-empty` 提示与 `.sftp-next` 按钮。
- **实际结果**：
  空目录被标记为 `has_more: true`，UI 同时展示“目录为空”和“下一页”按钮。
- **修复原则**：
  `page()` 内应使用 `while cursor.pending.len() < DIRECTORY_PAGE_ENTRIES && !cursor.ended` 循环拉取服务端包并填充 `cursor.pending`，直到填满单页容量或遇到真正的 EOF，再计算 `has_more`。

---

### 缺陷 2：双击无法进入指向目录的符号链接（UI 硬编码过滤 `kind !== "directory"`）

- **缺陷类别**：边界条件 / 功能契约不符
- **代码位置**：`ui/src/sftp/SftpPanel.tsx` lines 84–88
- **机理分析**：
  在 `SftpPanel.tsx` 中：
  ```typescript
  const openEntry = (entry: typeof selected): void => {
    if (!entry || entry.metadata.kind !== "directory") return;
    const base = state.page?.path ?? "/";
    void navigate(base === "/" ? `/${entry.name}` : `${base}/${entry.name}`);
  };
  ```
  在 `capstan-core::sftp::FileKind` 中，POSIX mode `0120000` 被准确分类为 `FileKind::Symlink`。
  因此，当目录列表中存在指向某个目录的符号链接条目时（如 Linux 系统常见的 `/var/mail`、`/var/lock` 或构建软链接），其 `entry.metadata.kind` 为 `"symlink"`。
  `openEntry` 包含守卫 `if (!entry || entry.metadata.kind !== "directory") return;`，导致双击符号链接时被直接忽略，用户无法通过双击进入该链接目录。
  这直接违反了 [`M2.md`](M2.md) §4.4 的硬性规则：“**浏览时可明确进入符号链接指向的目录**”。
- **复现步骤**：
  1. 在 UI 测试或真实 SFTP 列表中挂载一个 `metadata.kind === "symlink"` 且目标为目录的条目。
  2. 双击该条目。
  3. 观察网络/调用日志：`list_remote_directory` 永远不会被触发。
- **实际结果**：
  双击无任何响应，无法通过界面进入符号链接目录。
- **修复原则**：
  在 UI 双击或打开条目逻辑中，允许对 `kind === "directory" || kind === "symlink"` 执行导航尝试，远端若为目录则由 `realpath` 成功解析并进入；若为文件则按文件处理或报错。

---

### 缺陷 3：目录导航失败提前调用 `begin_directory` 清空条目缓存，导致界面上可见条目的后续操作均报 `scope_expired`

- **缺陷类别**：错误路径 / 状态不一致 / 可用性破坏
- **代码位置**：`crates/capstan-core/src/sftp/service.rs` 中的 `list()` (lines 251–280) 及 `crates/capstan-core/src/sftp/scope.rs` 中的 `begin_directory()`
- **机理分析**：
  在 `service.rs` 中：
  ```rust
  let revision = self
      .state
      .lock()
      .map_err(|_| SftpError::ConnectionLost)?
      .scopes
      .begin_directory(session, &view.binding, path.clone(), Instant::now())?;
  ...
  let canonical = RemotePath::parse(
      &view
          .client
          .realpath(path.as_str(), &cancel)
          .await
          .map_err(map_error)?,
  )?;
  ```
  1. 当用户在路径栏输入一个新路径并点击跳转时，`service.list()` 在尚未调用 `realpath` 或 `open_directory` 验证该路径是否存在、是否有权访问之前，就首先调用了 `scopes.begin_directory()`。
  2. `begin_directory()` 会递增 `revision`，并执行 `scope.entries.clear()`，清空当前作用域的所有缓存条目。
  3. 随后，若 `realpath` 或 `open_directory` 报错（例如路径不存在 `NoSuchFile`、权限不足 `PermissionDenied`、目标不是目录等），`list()` 提前返回 `Err`。
  4. 在前端 `controller.ts` 中，由于 `list()` 抛出异常，`store.setState({ page })` 不会更新，界面仍然停留在并展示之前的目录内容（如旧目录的 10 个文件）。
  5. 此时用户看到这 10 个文件，点击其中一个并尝试删除、重命名或查看权限，前端调用 `prepare_remote_mutation`。
  6. 后端 `prepare_remote_mutation` 调用 `resolve_entry(session, &view.binding, entry)` 时，发现 `scope.entries` 已经被第 2 步清空！
  7. `resolve_entry` 立即返回 `Err(SftpError::ScopeExpired)`。
  8. 结果前端弹出报错：“文件视图已过期，请重新连接 SFTP 获取当前目录。”用户可见界面的所有文件操作全部瘫痪，必须手动点击刷新重新拉取。
- **复现步骤与代码验证**：
  在 `crates/capstan-core/src/sftp/tests.rs` 中运行如下逻辑：
  ```rust
  let mut registry = ScopeRegistry::default();
  let now = Instant::now();
  let owner = binding();
  let id = registry.attach(owner.clone(), now).unwrap();
  let rev = registry.begin_directory(id, &owner, RemotePath::parse("/dir").unwrap(), now).unwrap();
  let page = registry.accept_page(id, &owner, PageInput {
      request_revision: rev,
      page_sequence: 0,
      entries: vec![(RemoteName::from_bytes(b"file").unwrap(), metadata())],
      has_more: false,
  }, now).unwrap();
  let entry_id = page.entries[0].id;
  assert!(registry.resolve_entry(id, &owner, entry_id, now).is_ok());

  // 模拟导航到一个不存在的目录，导航在 realpath 处失败：
  let _ = registry.begin_directory(id, &owner, RemotePath::parse("/invalid").unwrap(), now).unwrap();
  // 此时未调用 accept_page，导航已失败。但此时旧条目已丢失：
  let result = registry.resolve_entry(id, &owner, entry_id, now);
  assert_eq!(result, Err(SftpError::ScopeExpired));
  ```
- **实际结果**：
  原本仍然显示在用户界面的条目，因为一次失败的路径跳转尝试而全部变成 `ScopeExpired` 无法操作。
- **修复原则**：
  导航时应在成功完成 `realpath` 和 `open_directory` 首包准备后再推进 `scope.revision` 并清理旧条目，或者在导航失败时保留旧作用域条目。

---

### 缺陷 4：远端不存在路径（`NotFound`）被错误映射为 `RemoteIo`，产生误导性故障文案

- **缺陷类别**：错误路径 / 诊断误导 / 契约不符
- **代码位置**：`crates/capstan-core/src/sftp/service.rs` 中的 `map_error()` (lines 565–578)
- **机理分析**：
  在 `service.rs` 中：
  ```rust
  fn map_error(error: ClientError) -> SftpError {
      match error {
          ClientError::Unavailable => SftpError::SftpUnavailable,
          ClientError::PermissionDenied => SftpError::PermissionDenied,
          ClientError::InvalidPath => SftpError::PathInvalid,
          ClientError::Unsupported => SftpError::UnsupportedFileType,
          ClientError::ConnectionLost => SftpError::ConnectionLost,
          ClientError::Canceled => SftpError::OperationCanceled,
          ClientError::ResourceLimit => SftpError::ResourceLimit,
          ClientError::Uncertain | ClientError::NotFound | ClientError::RemoteIo => {
              SftpError::RemoteIo
          }
      }
  }
  ```
  `ClientError::NotFound` 来源于 SFTP 协议的 `StatusCode::NoSuchFile`。
  而在 `map_error` 中，`ClientError::NotFound` 被与 `Uncertain`、`RemoteIo` 合并，统一返回 `SftpError::RemoteIo`。
  根据 `ui/src/sftp/copy.ts`：
  - `remote_io` 文案：`"远程操作失败或结果不确定，请刷新核实后再重试修改。"`
  当用户在路径栏中输入一个拼写错误的路径（例如 `/var/logg`）并提交时，服务端返回 `NoSuchFile`，前端呈现的不是“路径不存在”，而是极具恐慌感且可操作性错误的提示：“远程操作失败或结果不确定，请刷新核实后再重试修改”。
  这直接违反了 [`M2.md`](M2.md) §8 验收清单第 3 项：“**无权限/不存在路径报错明确**”。
- **复现步骤**：
  在面板路径栏中输入 `/non_existing_dir_123` 并点击“前往/打开”。
- **实际结果**：
  前端弹错为 `remote_io`，文案为“远程操作失败或结果不确定...”，而非明确指出路径不存在。
- **修复原则**：
  `SftpError` 或错误映射体系中应有针对不存在目标的准确稳定错误（如复用 `path_invalid` 或明确定义 `not_found`），提供行动明确的用户文案。

---

### 缺陷 5：对符号链接执行 chmod 权限修改在准备阶段通过并弹出确认窗，但在确认应用阶段报错失败

- **缺陷类别**：错误路径 / 交互逻辑缺陷
- **代码位置**：`crates/capstan-core/src/sftp/service.rs` 中的 `prepare_remote_mutation()` (lines 412–471) 与 `apply_remote_mutation()` (lines 523–528)
- **机理分析**：
  在 `apply_remote_mutation` 中明确规定：
  ```rust
  Mutation::Permissions { mode } => {
      // SFTP v3 SETSTAT follows symlinks. Never chmod a link target
      // through a confirmation describing only the link itself.
      if current.kind == FileKind::Symlink {
          return Err(SftpError::UnsupportedFileType);
      }
  ...
  ```
  因为 SFTP v3 `SETSTAT` 会跟随符号链接，为防止误改目标文件的权限，禁止对符号链接执行 chmod。
  然而在 `prepare_remote_mutation` 中：
  ```rust
  if baseline.kind == FileKind::Unknown || baseline.kind == FileKind::Other {
      return Err(SftpError::UnsupportedFileType);
  }
  ```
  `prepare_remote_mutation` **根本没有检查** `baseline.kind == FileKind::Symlink` 且 `mutation` 为 `Permissions` 的情况！
  因此：
  1. 用户选中符号链接，点击工具栏“权限”按钮，输入 `0755`。
  2. `prepare_remote_mutation` 顺利通过，生成了有效的 `Confirmation` 令牌并返回给前端。
  3. 前端弹出完整的确认弹窗：“是否确认在主机 XXX 上对 /path/to/link (链接) 执行 将权限从 0777 改为 0755？”
  4. 用户点击“确认”后，前端调用 `apply_remote_mutation`。
  5. 后端此时才拦截 `if current.kind == FileKind::Symlink`，抛出 `unsupported_file_type` 错误。
  6. 弹窗内提示错误：“此名称或文件类型暂不支持，操作未执行。可在终端中检查。”
  整个准备和确认交互流程完全多余且具有误导性。
- **复现步骤**：
  1. 选中任意符号链接条目。
  2. 点击“权限”并输入八进制模式。
  3. 点击提交，观察到弹窗进入“确认修改”阶段。
  4. 点击确认，观察到操作报错失败。
- **实际结果**：
  必然失败的操作通过了准备阶段并向用户索取了确认，确认后报错。
- **修复原则**：
  在 `prepare_remote_mutation` 中提前拦截 `matches!(mutation, Mutation::Permissions { .. }) && baseline.kind == FileKind::Symlink` 并拒绝；同时在前端若选中项为符号链接，直接禁用“权限”操作按钮。

---

### 缺陷 6：目录下存在任一非 UTF-8 或包含替代符的文件名时，`?` 导致整个目录列表完全无法浏览

- **缺陷类别**：边界条件 / 健壮性缺陷 / 容错缺失
- **代码位置**：`crates/capstan-core/src/sftp/service.rs` line 336
- **机理分析**：
  在 `service.rs` 的目录页读取逻辑中：
  ```rust
  for (name, attributes) in names {
      if name == "." || name == ".." {
          continue;
      }
      cursor.pending.push_back((
          RemoteName::from_library_string(&name)?,
          metadata(attributes),
      ));
  }
  ```
  注意 `RemoteName::from_library_string(&name)?` 上的 `?`。
  根据 `path.rs`：
  ```rust
  pub fn from_library_string(value: &str) -> Result<Self, SftpError> {
      if value.contains('\u{fffd}') {
          return Err(SftpError::UnsupportedFileType);
      }
      Self::from_bytes(value.as_bytes())
  }
  ```
  `russh-sftp` 对非 UTF-8 文件名采用有损替换（生成 `\u{fffd}`）。
  当目标远程目录（例如用户的 `/tmp` 或包含旧 GBK/EUC-JP 文件名的目录）中包含**哪怕一个**无法被解析的条目时：
  `from_library_string` 返回 `Err(SftpError::UnsupportedFileType)`。
  这会导致当前 `page()` 调用立即因为 `?` 发生短路崩溃，整个 `list()` 彻底失败返回错误。
  结果是：用户因为目录里存在一个特殊文件，整个目录的上百个合法文件全部无法列出和查看。
  这违背了 [`M2.md`](M2.md) §4.4 中“**越界项单独报告**”及“清楚标记不支持，拒绝相关操作并记录兼容限制”的原则。
- **复现步骤**：
  在测试 SFTP 服务端目录中放置一个非 UTF-8 文件名（或者模拟 `readdir` 返回包含 `\u{fffd}` 的条目），使用客户端浏览该目录。
- **实际结果**：
  整个目录加载失败并报 `unsupported_file_type`，没有任何文件被呈现。
- **修复原则**：
  对解析失败的条目应进行单独降级标记（如保留为不可操作的占位条目或在当前视图中跳过并记录告警），而非直接熔断终止整个目录的读取。

---

### 缺陷 7：`create_remote_entry` 缺少当前浏览目录的作用域校验，允许 WebView 越权在任意未浏览路径创建文件或目录

- **缺陷类别**：安全隔离 / 作用域校验缺失
- **代码位置**：`crates/capstan-core/src/sftp/service.rs` 中的 `create()` (lines 385–411) 与 `src-tauri/src/sftp_commands.rs` (lines 85–105)
- **机理分析**：
  在 `service.rs` 中：
  ```rust
  pub async fn create(
      &self,
      window: &str,
      terminal: &str,
      session: SftpSessionId,
      parent: RemotePath,
      name: &str,
      directory: bool,
  ) -> Result<(), SftpError> {
      let view = self.view(window, terminal, session)?;
      let path = parent.join(&RemoteName::from_bytes(name.as_bytes())?)?;
      let cancel = CancellationToken::new();
      if directory {
          view.client.mkdir(path.as_str(), &cancel).await.map_err(map_error)
      } else {
          view.client.open_file(path.as_str(), true, &cancel).await.map_err(map_error)?.close(&cancel).await.map_err(map_error)
      }
  }
  ```
  对照重命名、删除与权限修改接口：
  - `rename`、`delete`、`permissions` 均强制要求提交 `RemoteEntryId`，并通过 `scopes.resolve_entry(session, &view.binding, entry)` 核验，确保操作对象**必须是当前会话合法浏览并缓存的条目**。
  - 然而，`create` 接口直接接收 WebView 传入的 `parent: RemotePath` 参数，**从未核验 `parent` 是否与该会话当前浏览的作用域路径一致**！
  - 恶意或异常的 WebView 脚本只要获得一个有效的 `session` ID，即可通过 `create_remote_entry` 命令在远程文件系统的**任意绝对路径**（如 `/root/.ssh`、`/etc/cron.d` 或其它未授权目录）创建文件或目录。
  这违背了 [`M2.md`](M2.md) §4.1 与 §4.3 中关于“路径授权、会话绑定、受控目标与防止能力逃逸”的安全边界原则。
- **复现步骤**：
  在当前正在浏览 `/home/user` 的会话中，通过 Tauri invoke 调用 `create_remote_entry` 并指定 `parent: "/tmp/unrelated/path"`。
- **实际结果**：
  调用绕过当前浏览作用域直接执行创建成功，未进行目录绑定校验。
- **修复原则**：
  在 `service.create` 中，必须通过 `scopes` 核验 `parent` 必须全等于当前作用域正在浏览的目录路径；若不匹配则拒绝执行。

---

### 缺陷 8：应用确认失败时后端单次令牌已被消费，前端弹窗保持打开且未禁用确认按钮，用户再次点击确认被报为毫无上下文的 `scope_expired`

- **缺陷类别**：错误路径 / 交互体验缺陷
- **代码位置**：`crates/capstan-core/src/sftp/service.rs` line 485 与 `ui/src/sftp/SftpPanel.tsx` lines 194–199
- **机理分析**：
  在 `service.rs` 的 `apply_mutation` 中：
  ```rust
  let pending = self
      .state
      .lock()
      .map_err(|_| SftpError::ConnectionLost)?
      .confirmations
      .remove(&token)
      .ok_or(SftpError::ScopeExpired)?;
  ```
  `confirmations.remove(&token)` 在初次校验时就无条件从内存中将该令牌移除（保证 single-use 防重放）。
  如果后续操作因为目标冲突（`TargetConflict`，如远端文件已被其他进程并发修改）或网络波动而失败：
  - 后端返回失败；
  - 前端 `SftpPanel.tsx` 中：
    ```typescript
    void controller.apply(dialog.confirmation).then((applied) => {
        if (applied) closeDialog();
        else setSubmitting(false);
    });
    ```
  - 当 `applied === false` 时，弹窗**依然保持打开**，并展示报错（如“目标已变化或已存在”），但**重新启用了“确认”按钮**（`setSubmitting(false)`）。
  - 若用户未关闭弹窗，再次点击“确认”按钮，前端将使用同一个 `token` 再次调用 `apply_remote_mutation`。
  - 由于令牌在第一次请求时已被移除，后端直接返回 `Err(SftpError::ScopeExpired)`！
  - 弹窗内的错误信息突然突变为：“文件视图已过期，请重新连接 SFTP 获取当前目录。”
  这种状态突变给用户造成严重困惑。
- **复现步骤**：
  1. 触发一次必然在 `apply` 阶段失败的修改（如在 `prepare` 后并发修改该文件触发 `TargetConflict`）。
  2. 弹窗提示冲突后，不关闭弹窗，直接再次点击弹窗内的“确认”按钮。
- **实际结果**：
  错误信息从“目标已变化”突变为“文件视图已过期”。
- **修复原则**：
  `apply` 失败后，由于令牌已失效，前端应当强制关闭确认弹窗并刷新，或禁用“确认”按钮，提示用户重新发起修改流程。

---

### 缺陷 9：多页浏览直接替换当前页且立即释放前页条目，且无“上一页”导航

- **缺陷类别**：交互设计 / 分页状态维护缺陷
- **代码位置**：`ui/src/sftp/controller.ts` lines 47–53 及 `ui/src/sftp/SftpPanel.tsx` line 173
- **机理分析**：
  在 `controller.ts` 中：
  ```typescript
  const nextPage = await transport.next(attached.sessionId, page.requestRevision);
  if (!disposed && operation === generation && nextPage.sessionId === attached.sessionId
    && nextPage.requestRevision === page.requestRevision && nextPage.pageSequence === page.pageSequence + 1) {
    store.setState({ page: nextPage, busy: false });
    await transport.release(attached.sessionId, page.entries.map((entry) => entry.id)).catch(() => undefined);
  }
  ```
  在 `SftpPanel.tsx` 中，只渲染了：
  ```tsx
  {state.page?.hasMore && <button className="sftp-next" type="button" onClick={() => void controller.next()} disabled={state.busy}>{labels.next}</button>}
  ```
  1. 当目录文件数量较多触发分页时，用户点击“下一页”，前端收到 `nextPage` 并直接全量覆盖 `store.page`。
  2. 随后前端立即向后端请求 `transport.release()` 释放上一页的所有 `RemoteEntryId`。
  3. 界面中只保留了“下一页”按钮，**没有任何“上一页”按钮或滚动分页回滚机制**。
  4. 用户一旦翻到第 2 页，第 1 页的文件不仅在界面上彻底消失，后端的缓存也已被销毁；用户无法回到上一页，只能重新输入路径或点击刷新回到第 0 页重新翻。
- **复现步骤**：
  浏览一个超过 256 项的目录，点击“下一页”。
- **实际结果**：
  前 256 项消失，无返回上一页的任何入口。
- **修复原则**：
  B 门要求完整的大目录分页/虚拟化交互。要么在 UI 维护页码列表与前后翻页导航，要么采用虚拟滚动列表；且条目的释放范围需与前端可见视口精确同步。

---

## 4. 两道验收门独立核验结论

对照 [`M2.md`](M2.md) 第 5 节的验收标准，独立审查核验结果如下：

### 4.1 A 门：共享连接上的安全文件闭环

| 编号 | 验收项 | 审查状态 | 依据与缺口 |
| --- | --- | --- | --- |
| **A1** | SFTP 依赖可用，许可证/漏洞通过，性能与原生拖拽探针完成，数值预算冻结 | **部分满足（未验收）** | `russh-sftp 3.0.1` 依赖与许可通过；但应用级绝对性能阈值尚未冻结（`performance-policy.json` 中阈值为 null），原生拖拽仅为独立 probe，未接通应用工作流。 |
| **A2** | 同连接两 shell 打开单 SFTP，认证/连接计数/取消/空闲回收正确 | **部分满足（未验收）** | 进程内两 shell + 单 SFTP 测试存在；但连接池在真实断线、重试及失败隔离下的全量状态机矩阵仍待补齐。 |
| **A3** | 手动浏览、分页、基础文件操作正确，跨标签迟到事件和路径逃逸被拒绝 | **存在缺陷（未验收）** | 基础浏览和单项增删改已实现，跨标签 token 校验存在；但存在缺陷 1（空目录翻页）、缺陷 2（无法进入软链目录）、缺陷 3（导航失败缓存失效）、缺陷 5（软链 chmod 逻辑不一致）、缺陷 6（单损坏文件拖垮目录列表）、缺陷 7（创建路径未约束到当前浏览目录）。 |
| **A4** | 单文件双向传输可提交，覆盖冲突/取消/权限或磁盘失败保留原目标 | **未实现（0%）** | 核心与 UI **完全未实现单文件上传/下载**，无临时文件提交、覆盖冲突确认与故障保留。 |
| **A5** | 文件正文不经过 WebView，原生来源/目的地授权和最小命令能力成立 | **部分满足（未验收）** | 元数据不含明文或凭据，未走通用 fs；但选择器 Token 与受限落盘能力因传输未实现而未闭环。 |
| **A6** | Windows 可见 UI + WSL OpenSSH 完成连接、浏览、上传、下载、关闭的实际闭环 | **未验收（0%）** | 未执行真实 GUI 下的上传、下载、关闭闭环验证。 |

**A 门结论**：**未通过（`not accepted`）**。

---

### 4.2 B 门：完整工作流与候选交付

| 编号 | 验收项 | 审查状态 | 依据与缺口 |
| --- | --- | --- | --- |
| **B1** | 面板停靠/折叠、标签跟随、基础操作、路径和键盘/双语交互完整 | **部分实现（未验收）** | 停靠、折叠已实现；但停靠偏好未持久化，缺少键盘表格导航（缺少上下键移动与快捷键），存在缺陷 8、9。 |
| **B2** | 文件和目录双向传输/拖拽、全局 3 并发、流水线、进度/速度/ETA 正确 | **未实现（0%）** | 传输队列、多 worker 调度器、递归目录树、拖拽接线全部未开始。 |
| **B3** | 固定条件 1 GB 文件上传/下载中断续传，双方完整 SHA-256 一致 | **未实现（0%）** | 续传校验状态机与 1 GB 断点续传测试未实现。 |
| **B4** | 连续保存自动回传，远程编辑冲突保留双方，失败/退出/残留安全处理 | **未实现（0%）** | 本地受控编辑生命周期、文件监测与回传未实现。 |
| **B5** | OSC 7 会话隔离，Bash/Zsh 集成可重复安装/卸载且不覆盖用户配置 | **未实现（0%）** | OSC 7 解析接线与 shell 配置文件事务安装器未实现。 |
| **B6** | 与 WinSCP 同条件性能比较通过冻结阈值，终端/资源回收满足 M1 回归 | **未验收（0%）** | 无 WinSCP 真实对比数据，性能政策无绝对阈值。 |
| **B7** | 默认/runtime/全部特性、UI、安全/许可、真实 OpenSSH 与受影响桌面回归通过 | **部分满足（自动化基线）** | 当前源码自动化工程检查全过，但真实 OpenSSH 完整工作流与实机桌面验证待交付。 |
| **B8** | v0.2 安装/便携候选、升级保留数据、文档与维护者验收完整，board 逐项签收 | **未开始（0%）** | 未构建 v0.2 候选，维护者手测清单（§8）全部未勾选。 |

**B 门结论**：**未通过（`not accepted`）**。

---

## 5. 回归与安全审查

1. **M0/M1 核心回归**：
   - 验证了本地 ConPTY、WSL 发行版列表、PTY 尺寸变更与退出回收，所有单元及集成测试通过。
   - 验证了 `capstan-vault` 目录扫描配额与备份轮换，未受 SFTP 变动影响。
   - 验证了 `capstan-terminal` 原始二进制通道与背压水位线流控（1 MB 暂停，256 KB 恢复），流控机制未被文件传输侵入。
   - 验证了明文密码、私钥与服务器主机密钥的零化管理与隔离性。
2. **CSP 与权限审查**：
   - 前端生产构建 CSP 依然严格限制为 `'self'`，未放开外部脚本或跨域远程资源。
   - `sftp_commands.rs` 严格限制必须来自 `window.label() == "main"`，防止非法 WebView 上下文越权调用。

---

## 6. 审查建议与后续行动

1. **缺陷修复建议（聚焦当前已有代码，不改变架构）**：
   - **修复空目录分页**：将 `service.rs` 中的 `page()` 读取改造为带容量上界的循环拉取，确保在获取到真实 EOF 或填满单页前不提前返回空页及错误的 `has_more: true`。
   - **支持软链目录导航**：在 `SftpPanel.tsx` 的 `openEntry` 中放行 `kind === "symlink"`，允许触发进入目录。
   - **保护条目缓存一致性**：在 `service.list()` 中，将 `begin_directory()` 和旧条目清空推迟到 `realpath` / `open_directory` 校验成功之后；或在导航失败时保留旧条目，防止 visible items 失效。
   - **精确化错误映射**：区分 `ClientError::NotFound`，映射为清晰的路径不存在错误代码与本地化文案。
   - **对称权限修改检查**：在 `prepare_remote_mutation` 中提前拦截针对符号链接的 `Permissions` 变更，并在前端针对符号链接禁用修改权限按钮。
   - **提升目录容错性**：对解析失败的损坏条目单项记录或过滤，避免单条损坏文件名熔断整个目录的展示。
   - **收口创建路径作用域**：在 `service.create` 中，强制核验 `parent` 必须与当前会话的作用域目录路径严格一致，杜绝跨目录越权创建。
   - **优化失效令牌交互**：应用操作失败时，强制重置/关闭确认弹窗，防止用户误点重提死令牌引发误导性 `scope_expired`。
2. **后续实施顺序**：
   - 严格按照 [`M2.md`](M2.md) §6 规定的顺序：先完成 **M2-05**（单文件流式上传/下载、受控临时文件、原子提交与取消），实现 A 门闭环；
   - 再依序进入 M2-06（调度队列与拖拽）、M2-07（断点续传）、M2-08（远程编辑）、M2-09（OSC 7）与 M2-10（Shell 集成事务）；
   - 在各模块完成前，保持看板为 `in_progress`，不提前宣称里程碑交付。

---

## 7. 2026-10-21 独立复核（当前 M2 实现）

> 审查角色：C（独立审查与验证）。本节基于当前工作区源码、[`M2.md`](M2.md)、[`M2-B.md`](M2-B.md)、[`M2-CONTRACTS.md`](M2-CONTRACTS.md) 和实际测试结果，不将 B 报告的自查结论当作证据。  
> 结论：**新增 1 项高优先级恢复记录缺陷和 2 项低优先级边界/交互缺陷；独立复核不通过，M2/B 仍为 `blocked`。** 未修改架构、源码或 [`PLAN.md`](../PLAN.md)，未提交/推送。既有 A 门签收范围未被改写。

### 7.1 范围、方法与证据边界

- 逐项检查 `crates/capstan-core/src/transfer/{engine,native,tree,batch,recovery,mod}.rs`、`crates/capstan-core/src/sftp/{service,scope,delete,cwd}.rs`、`src-tauri/src/transfer_commands.rs`、`ui/src/transfers/*` 与 `ui/src/sftp/SftpPanel.tsx`。
- 除全量基线外，针对清理失败、并发上限重检、授权过期三条错误路径编写临时故障注入测试；测试运行后已删除，仓库现有测试文件恢复原 SHA-256，未留下源码改动。
- 证据边界：高优先级发现使用进程内 russh SFTP 夹具和真实服务代码复现；下载变体在本地目的地 trait 层注入 `discard` 失败；UI 发现使用组件测试模拟后端 5 分钟授权到期后的 `ready → retained` 状态转换，未真实等待 5 分钟。没有维护者桌面手测或真实杀毒软件/文件锁环境证据。
- **复审更正（同日）**：初稿曾把 UI 上传重试描述为通常返回 `scope_expired`，并称只有 `retain_partial` 会写恢复记录；还把上传冲突时的恢复记录误报为 1。再次核对 `TransferQueue.tsx`、`engine.rs` 和 `mod.rs` 后：UI 重试会重新打开本地选择器并调用 `retryWithSelection`；恢复记录在 OPEN/`begin` 前后由 `checkpoint_recovery` 提前写入，`retain_partial` 只在可续传时更新；但上传冲突发生在上传开始前，冲突时记录为 0，选择 `Replace` 后上传期间才写为 1，清理失败后删为 0。下文已按此更正，并把修复建议收紧为：只对仍能验证所有权的残留保留清理记录，避免把已外部替换的文件交给清理入口。

### 7.2 工程检查结果（本节实际执行）

| 检查项目 | 执行命令 | 实际结果 |
| --- | --- | --- |
| 默认工作区测试 | `cargo test --workspace` | 329 项通过，0 失败 |
| Tauri Runtime 测试 | `cargo test -p capstan-app --features tauri-runtime` | 14 项通过，0 失败 |
| 全特性测试 | `cargo test --workspace --all-features` | 351 项通过，0 失败 |
| 原生 Windows crate | `cargo test --manifest-path vendor/capstan-native-windows/Cargo.toml` | 30 项通过，0 失败 |
| 代码格式 | `cargo fmt --all -- --check` | 退出码 0 |
| 全特性 Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 退出码 0，零告警 |
| 前端类型检查 | `pnpm -C ui typecheck` | 退出码 0 |
| 前端测试 | `pnpm -C ui test` | 34 个文件 / 195 项通过，0 失败 |

说明：PowerShell 会把 `cargo` 的 stderr 进度输出包装成 `NativeCommandError`，有时显示为红色；上表结论以 cargo 自身 `test result`、日志和 `$LASTEXITCODE` 为准。各步日志位于 `%TEMP%\m2c-*.log`，其中 `m2c-workspace-allfeatures.log`、`m2c-native-test.log`、`m2c-clippy-final.log`、`m2c-ui-test-final.log` 为本轮最终记录。上述全绿不代表第 7.3 节路径已被覆盖；未设置 `CAPSTAN_TEST_SSHD` 时，配置型 OpenSSH/性能用例按既有逻辑条件跳过或返回空结果，本轮不将其计为真实外部执行证据。

### 7.3 新增发现

#### 7.3.1 高：清理失败但不可续传时，恢复记录被删除，残留文件不可追踪

- **类别**：错误路径 / 恢复记录一致性 / 资源残留。
- **代码位置**：
  - `crates/capstan-core/src/transfer/engine.rs` 第 1138–1168 行（`cleanup_required`、`retained_partial`、`clear_recovery`）；
  - 同文件第 799–897 行（`Job::cleanup` 与 `Job::partial_for_retry`）；上传和下载的提前记录分别在同文件第 468–502、703–709 行；
  - `crates/capstan-core/src/transfer/mod.rs` 第 374–449 行（`retain_partial`）和第 461–505 行（`checkpoint_recovery`）；
  - 同文件第 1057–1075 行（`cleanup_recovered_upload` 在完成路径/策略授权后，对临时文件本身只按类型和大小校验，不校验远端临时文件身份）。
- **机理**：
  1. `Job::cleanup()` 在远端 `remove_file` 或本地 `discard()` 失败时返回 `true`，即 `cleanup_required = true`；此时任务所有的远端临时文件或本地 `.partial` 仍可能存在。
  2. `Job::partial_for_retry()` 只有在 `task.cancel.is_cancelled()`、连接已关闭或 `self.partial.is_some()` 时才返回部分状态。一次“连接仍正常、任务未取消、提交因权限被拒绝”的失败不满足这些条件，因此返回 `None`。
  3. 恢复记录不是等到失败后才写：上传在冲突决策后、OPEN 前后，下载在 `begin` 后就会通过 `checkpoint_recovery` 写入；只有可续传时 `retain_partial` 才更新它。随后 `run()` 执行 `if result.is_ok() || !retained_partial { task.clear_recovery() }`。由于 `retained_partial == false`，即使 `cleanup_required == true` 且 checkpoint 记录仍在，记录也会被删除；本次失败不会再有后续写入。
  4. 结果：状态行显示“需要清理”，但恢复列表为空；`TransferQueue` 的清理按钮只对恢复列表行渲染，失败任务行只有“重试”。上传失败还会在 `run()` 第 1155–1161 行清空重试 plan 的本地来源；UI 的 `retryTask` 会重新打开本地选择器并调用 `retryWithSelection`（`TransferQueue.tsx` 第 61–75 行）；在本路径下没有可续传的 partial，因此重试是重新选择源后从头开始，不会复用或清理旧残留，也不会因本地来源被清空而直接返回 `scope_expired`。旧残留仍既不能续传，也不能从界面清理。
- **违反的约定**：[`M2.md`](M2.md) 第 85 行要求异常退出残留可发现且可安全清理；§4.6 第 181、183 行要求原任务保存受控临时文件标识与已确认范围，身份无法确认时阻断并提供重传/改名/跳过。本路径在清理失败后删除记录，连残留标识都不再保留，既不能续传也不能清理；§4.7 第 201 行“可验证所有权、未清理结果可追踪”是同类约束。
- **上传复现（临时测试，已删除）**：
  - 在 `crates/capstan-core/tests/sftp_workflow/transfers.rs` 追加测试，配置恢复目录，向已存在的 `/files/target` 上传 `b"new"`，设置 `remote.deny_commit = true`，并把 `/files/.capstan-{id}.partial` 放入 `remote.denied_removals`。
  - 冲突检测发生在上传前：等待冲突时恢复记录为 0。选择 `Decision::Replace` 后上传，checkpoint 才在 OPEN 前后写入；另设 `remote.stall_write = true`，在 resolve 后等待首个写请求，快照为 1。
  - 释放写入后终态 `Failed`、`cleanup_required == true`、拥有的临时文件仍为 `b"new"`，最后快照恢复记录为 0。旧版最终断言（期望终态仍为 1）也在临时行 1726 失败：`left: 0, right: 1`。
  - 实际输出（观察测试）：
    ```
    recovery_at_conflict=0
    recovery_during_upload=1
    after_failure state=Failed cleanup_required=true recovery=0 retained=Some([110, 101, 119])
    test result: ok. 1 passed; 0 failed
    ```
    即记录是在上传期间写入、清理失败后被删除，而不是冲突时已经存在。
  - 运行命令：`cargo test -p capstan-core --test sftp_workflow m2c_owned_cleanup_failure_must_keep_recovery_record -- --nocapture`（观察版测试名为 `m2c_observe_recovery_between_checkpoint_and_cleanup_failure`）
- **下载变体（同一根因，临时测试，已删除）**：
  - 使用现有 `FaultDestination`，令 `commit` 返回 `PermissionDenied`、`discard` 返回 `LocalIo`，下载到新目标后等待终态。源码在 `destination.begin` 后写 checkpoint；测试观察到终态为 0，未单独停住传输去读取中间值。
  - 实际输出：
    ```
    cleanup_required=true
    retained_local_partial=C:\Users\...\.capstan-812ed5f9-5947-4304-af42-9772778d8aa8.partial
    recovery_entries_after=0
    assertion `left == right` failed
      left: 0
     right: 1
    ```
    即本地 `.partial` 仍在、`cleanup_required=true`，终态恢复记录为 0；按源码中的 checkpoint 写入路径属于写入后删除，但未单独观测下载中间值。
- **影响**：远端临时文件可能包含用户上传的完整或部分内容，却不再出现在恢复列表；本地 `.partial` 也可能占用磁盘并保留用户数据。两者都只能靠用户手工定位和删除；恢复记录删除后，UI 无法再针对该残留执行续传或清理，违背“可追踪”的要求。触发条件不是正常取消/断线（这些路径已有测试），而是“清理动作本身失败”的权限/占用类错误，属于必须覆盖的错误路径。
- **修复方向（不改变架构）**：不能只把 `cleanup_required` 当作保留恢复记录的条件。该布尔值至少把“拥有权仍可验证的删除失败”和“临时文件已被外部替换、`lstat` 失败或 OPEN 结果不确定”折叠为同一个 true；而清理入口 `cleanup_recovered_upload` 对临时文件本身只检查常规文件和大小范围（第 1057–1075 行，`valid_cleanup_size` 见 `recovery.rs` 第 353–358 行）。若给外部替换文件保留恢复记录，大小恰好符合条件的替换内容可能被当作本任务残留删除。应让 `Job::cleanup()` 区分结果（例如：已清理 / 可验证拥有的残留清理失败 / 所有权不可验证或已变化），只对第二类保留或写入清理专用的恢复描述，并为远端临时文件记录清理所需的属性基线（清理时再次比较）；SFTP v3 仍无法消除最终检查到删除之间的竞态。第三类若需保留可追踪性，可写入仅展示的人工检查描述并隐藏/拒绝清理动作；至少不能沿用当前可执行清理的恢复记录。同时为“拥有的临时文件删除被拒”“本地 `discard` 失败”和“外部替换的临时文件不得获得清理权”各加一条回归。现有测试 `failed_upload_cleanup_preserves_a_replaced_remote_temporary` 只覆盖“外部替换未被删除且 `cleanup_required` 为真”，且未配置恢复目录，不能覆盖本路径或清理入口的授权边界。

#### 7.3.2 低：并发导出超过上限时，重检失败会遗留孤立暂存目录

- **类别**：并发边界 / 本地资源泄漏。
- **代码位置**：`crates/capstan-core/src/transfer/native.rs` 第 390–407 行；同文件第 311–329 行（重启时发现暂存目录）；`vendor/capstan-native-windows/src/export.rs` 第 59–106 行（`ExportStage::prepare` 创建目录和标记，类型没有 `Drop` 清理）。
- **机理**：`prepare_native_export` 先在锁内检查 `MAX_EXPORTS`，释放锁后通过 `ExportStage::prepare` 创建暂存目录，再在第二次锁内重检上限。两个并发调用都可能通过第一次检查并各自创建目录，其中一个在第二次检查时返回 `ResourceLimit`，但没有对刚创建的 stage 调用 `cleanup()`。该目录带合法 `.capstan-export` 标记，进程重启后会被 `configure_export_directory` 当作保留导出发现。
- **复现（临时测试，已删除）**：
  - 在 `crates/capstan-core/tests/sftp_workflow/native_transfers.rs` 追加测试，配置导出目录，对同一个 `/files/one.bin` 用 `tokio::join!` 同时发起 9 次 `prepare_native_export`。
  - 断言：8 次成功、1 次 `ResourceLimit`；暂存目录数为 9；释放 8 个成功导出后目录数为 1；丢弃服务并用同一目录新建 `TransferService` 后，`export_snapshot("main")` 仍有 1 项。
  - 实际结果：复现测试通过（`1 passed; 0 failed`），上述计数全部成立；这是把当前错误行为写成断言的复现，不表示缺陷已修复。运行命令：`cargo test -p capstan-core --test sftp_workflow m2c_concurrent_export_limit_leaves_no_orphan_stage -- --nocapture`。
- **影响**：孤立目录只含 `.capstan-export` 标记，不含导出内容（创建后尚未配置文件），但会占用重启后的 `MAX_EXPORTS` 名额；若反复触发且未清理，可能把真正需要保留的导出挤出前 8 项。本次单个孤立的复现中，重启后它会以 UUID 名称出现在保留导出列表（`NativeExports`），可从该入口清理，但在用户清理前仍占用名额和磁盘。严重度低，但属于可避免的资源泄漏。
- **修复方向**：在第二次检查失败时先对 `stage` 执行有界清理（并处理清理失败），或把上限预留/插入与 stage 创建放在同一所有权流程中；也可让空的、尚未配置的 `ExportStage` 在丢弃时自动删除。补充并发上限回归。

#### 7.3.3 低：SFTP 面板的导出按钮在 5 分钟授权过期后仍显示可拖拽

- **类别**：前端状态过期 / 错误提示。
- **代码位置**：`ui/src/sftp/SftpPanel.tsx` 第 140–147 行（只在 `planning`/`preparing` 时轮询）；`crates/capstan-core/src/transfer/native.rs` 第 776–824 行（`ready` 超过 `LOCAL_GRANT_LIFETIME_SECONDS` 后返回 `retained`）；同文件第 642–662 行（非 `ready` 时 `begin_export_drag` 返回 `ScopeExpired`）；`ui/src/transfers/NativeExports.tsx` 第 15–21 行（独立每秒轮询）。
- **机理**：面板轮询在 `exportItem.state` 变为 `ready` 后停止。后端在下一次快照时发现 `ready_at` 已超过 5 分钟并返回 `retained`；面板自身不再读取该快照，按钮仍以本地 `state === "ready"` 渲染为可拖拽。用户拖拽时 `start_native_export` 才被调用，后端返回 `scope_expired`，界面显示“文件视图已过期”。
- **复现（临时组件测试，已删除）**：
  - 在 `ui/src/sftp/M2C-export-stale.test.tsx` 中 mock `prepare_native_export` 返回 `planning`，`resolve_transfer_tree_plan` 返回 `queued: 1`，`native_export_snapshot` 先返回 `ready`，随后切换为 `retained`；记录 `native_export_snapshot` 调用次数。
  - 断言：`ready` 后按钮为 `Drag to Explorer · notes.txt` 且 `draggable=true`；切换为 `retained` 并等待 700 ms 后快照调用次数不再增加，按钮仍是同一个可拖拽节点，界面没有 `Retained copy`；触发 `dragstart` 后调用 `start_native_export`，并显示 `scope_expired` 文案。
  - 实际结果：复现测试 `1 passed`，上述断言全部成立；通过表示当前缺陷行为被复现，不表示修复。运行命令：`pnpm -C ui exec vitest run src/sftp/M2C-export-stale.test.tsx --reporter=verbose`。
- **影响**：过期后的面板控件误导用户，拖拽会失败并显示与“文件视图过期”有关的错误；按 `NativeExports.tsx` 的独立每秒轮询实现（代码核对，本次组件测试未同时挂载该组件），它会显示“已保留副本”，两个入口状态不一致。不造成数据丢失，严重度低。
- **修复方向**：在 `exportItem` 存在且未释放时继续低频轮询（或由后端推送状态），在 `retained`/`failed` 时禁用控件；若坚持在 `dragstart` 时刷新，需要避免异步等待破坏原生拖拽的启动时序。补充“ready → retained”状态转换的组件回归。

### 7.4 回归、非发现与剩余缺口

- **无基线回归**：第 7.2 节全部通过，未发现既有功能退化。新增问题都是当前测试集未覆盖的错误路径；仓库没有 Git 历史，本轮无法做版本差分，因此不对“由哪次改动引入”下结论。
- **非发现（已核对，不作为缺陷）**：
  - `SftpPanel` 确认对话框 `apply().then(closeDialog)` 在失败时也关闭，是“失败操作同样消费令牌”的既有设计，避免重复提交死令牌。
  - `sftp/service.rs` 的 `lstat` 后再 `rename` 存在外部替换窗口，B 报告已将其列为 SFTP v3 限制，本轮不重复计为新发现。
  - 目录树拒绝符号链接/非 UTF-8 名称是 B 报告的已声明限制，不属于本轮新增问题。
- **剩余缺口**：
  - 未做真实 5 分钟墙钟等待，UI 发现使用后端状态模拟；未在真实杀毒软件/文件锁/只读磁盘环境验证 `discard` 失败。
  - 未验证孤立导出目录长期累积到 8 个以上时的启动挤出行为；当前只证明单个孤立的产生和重启后可发现。
  - 未执行维护者桌面手测、Explorer 实际拖拽和真实 OpenSSH 断线场景；这些仍按 [M2-MANUAL](M2-MANUAL.md) 和 board 的要求保留。
  - 修复后应重跑第 7.2 节全部检查，并补上第 7.3 节修复方向中列出的回归；在维护者确认前，**不将本轮发现标记为已修复，也不改变 B/M2 的 `blocked` 状态**。
- **仓库状态**：临时测试已删除；`crates/capstan-core/tests/sftp_workflow/transfers.rs` 恢复为 `788592762926536EEA5FDAD1FD42D9A2ADB6C2528DF3438FF4899BEE517B65CF`，`native_transfers.rs` 恢复为 `18443D16C7DD7A8F6469CEAD6B9D3A9FCA05E14138495F5DF89148C4BDECA0FE`，UI 临时测试文件不存在。除本文件外未改动其他文件。

### 7.5 临时复现代码（供维护者复核，不属于当前测试集）

以下代码仅用于本轮复现，运行后已删除；重新加入对应文件后可复跑第 7.3 节命令。代码中的行号是追加时的临时行号，不代表仓库现有行号。注意：7.5.1/7.5.2 断言的是期望行为，运行会失败；7.5.3/7.5.4 把当前错误行为写成断言，运行通过只表示缺陷被复现，不表示修复。

<details>
<summary>7.5.1 上传：拥有的临时文件删除被拒后恢复记录丢失</summary>

```rust
#[cfg(windows)]
#[tokio::test]
async fn m2c_owned_cleanup_failure_must_keep_recovery_record() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/target", b"original");
    remote.deny_commit.store(true, Ordering::SeqCst);
    remote.stall_write.store(true, Ordering::SeqCst);
    let recovery = tempfile::tempdir().unwrap();
    transfers
        .configure_recovery_directory(recovery.path().to_owned())
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::write(&source, b"new").unwrap();
    let id = enqueue_upload(&fixture, session, &transfers, &source, "target");
    let temporary = format!("/files/.capstan-{id}.partial");
    remote.denied_removals.lock().unwrap().insert(temporary.clone());
    let conflict = wait(&transfers, id, |s| s.conflict.is_some())
        .await
        .conflict
        .unwrap();
    // The conflict is detected before upload, so no checkpoint exists yet.
    assert_eq!(transfers.recovery_snapshot().unwrap().entries.len(), 0);
    transfers
        .resolve("main", id, conflict.revision, Decision::Replace)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), remote.started.notified())
        .await
        .unwrap();
    assert_eq!(transfers.recovery_snapshot().unwrap().entries.len(), 1);
    remote.stall_write.store(false, Ordering::SeqCst);
    remote.release.notify_waiters();
    let status = finished(&transfers, id).await;
    assert_eq!(status.state, TransferState::Failed, "{status:?}");
    assert!(status.cleanup_required);
    let retained = remote.files.lock().unwrap().get(&temporary).map(|f| f.data.clone());
    assert_eq!(retained.as_deref(), Some(&b"new"[..]));
    assert_eq!(transfers.recovery_snapshot().unwrap().entries.len(), 1); // 实际为 0
    fixture.close().await;
}
```

追加到 `crates/capstan-core/tests/sftp_workflow/transfers.rs`；运行 `cargo test -p capstan-core --test sftp_workflow m2c_owned_cleanup_failure_must_keep_recovery_record -- --nocapture`。上述代码保留期望行为，最后的 `assert_eq!(... , 1)` 会失败；7.3.1 的观察版把最后断言改为 0，并在冲突、上传中和终态打印计数。

</details>

<details>
<summary>7.5.2 下载：discard 失败后恢复记录丢失</summary>

在现有 `FaultDestination` 的 `discard` 中临时加入 `if self.fault == 2 { Err(SftpError::LocalIo) } else { self.inner.discard() }`，然后追加：

```rust
#[cfg(windows)]
#[tokio::test]
async fn m2c_local_discard_failure_must_keep_recovery_record() {
    let (fixture, session, transfers) = fixture().await;
    fixture.observations.transfer.put("/files/source", b"remote content");
    let recovery = tempfile::tempdir().unwrap();
    transfers.configure_recovery_directory(recovery.path().to_owned()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let entry = fixture.list(session, "/files").await.unwrap().entries[0].id;
    let context = fixture.service
        .transfer_context("main", "one", session, TransferTarget::Download { entry })
        .unwrap();
    let destination = FaultDestination {
        inner: capstan_core::transfer::WindowsDestination::new(
            capstan_native_windows::transfer::Destination::select(&output).unwrap(),
        ),
        fault: 2,
        hold: None,
    };
    let selection = transfers.grant_destination(&context, Arc::new(destination)).unwrap();
    let id = transfers.enqueue(context, selection.token).unwrap();
    let status = finished(&transfers, id).await;
    assert_eq!(status.state, TransferState::Failed, "{status:?}");
    assert!(status.cleanup_required);
    assert_eq!(transfers.recovery_snapshot().unwrap().entries.len(), 1); // 实际为 0
    fixture.close().await;
}
```

</details>

<details>
<summary>7.5.3 并发导出上限重检遗留孤立 stage</summary>

```rust
#[tokio::test]
async fn m2c_concurrent_export_limit_leaves_no_orphan_stage() {
    let (fixture, session, transfers) = fixture().await;
    let remote = &fixture.observations.transfer;
    remote.put("/files/one.bin", b"one");
    let page = fixture.list(session, "/files").await.unwrap();
    let entry = page.entries.iter().find(|entry| entry.name == "one.bin").unwrap().id;
    let staging = tempfile::tempdir().unwrap();
    transfers.configure_export_directory(staging.path().to_owned()).unwrap();
    let prepare = || transfers.prepare_native_export(&fixture.service, "main", "one", session, entry);
    let (a, b, c, d, e, f, g, h, i) = tokio::join!(
        prepare(), prepare(), prepare(), prepare(), prepare(), prepare(), prepare(), prepare(), prepare()
    );
    let results = [a, b, c, d, e, f, g, h, i];
    let mut ok = Vec::new();
    let mut limits = 0;
    for result in results {
        match result {
            Ok(value) => ok.push(value),
            Err(SftpError::ResourceLimit) => limits += 1,
            Err(error) => panic!("unexpected export error: {error:?}"),
        }
    }
    assert_eq!(ok.len(), 8);
    assert_eq!(limits, 1);
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 9);
    for value in ok {
        transfers.release_native_export("main", value.export.token).await.unwrap();
    }
    assert_eq!(std::fs::read_dir(staging.path()).unwrap().count(), 1); // 应为 0
    drop(transfers);
    let restarted = TransferService::default();
    restarted.configure_export_directory(staging.path().to_owned()).unwrap();
    assert_eq!(restarted.export_snapshot("main").unwrap().len(), 1); // 孤立项被重启发现
    fixture.close().await;
}
```

追加到 `crates/capstan-core/tests/sftp_workflow/native_transfers.rs`；运行 `cargo test -p capstan-core --test sftp_workflow m2c_concurrent_export_limit_leaves_no_orphan_stage -- --nocapture`。

</details>

<details>
<summary>7.5.4 UI：ready → retained 后按钮仍可拖拽</summary>

临时文件 `ui/src/sftp/M2C-export-stale.test.tsx` 的关键 mock 与断言：

```tsx
if (command === "prepare_native_export") return {
  export: { token: exportToken, name: "notes.txt", state: "planning" },
  plan: { token: planToken, revision: "0", name: "notes.txt", files: 1, conflicts: [] },
};
if (command === "resolve_transfer_tree_plan") return { plan: null, queued: 1 };
if (command === "native_export_snapshot") return [{ token: exportToken, name: "notes.txt", state: exportState }];
if (command === "start_native_export") throw { code: "scope_expired" };

// 选中 notes.txt、点击 Prepare Explorer export，等待第一次 ready 轮询
await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 650)); });
const ready = button("Drag to Explorer · notes.txt");
expect(ready?.disabled).toBe(false);
expect(ready?.draggable).toBe(true);
const snapshots = mocks.invoke.mock.calls.filter(([command]) => command === "native_export_snapshot").length;

// 模拟后端 5 分钟到期
exportState = "retained";
await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 700)); });
expect(mocks.invoke.mock.calls.filter(([command]) => command === "native_export_snapshot").length).toBe(snapshots);
expect(button("Drag to Explorer · notes.txt")).toBe(ready);
expect(button("Retained copy · notes.txt")).toBeUndefined();

await act(async () => { ready?.dispatchEvent(new Event("dragstart", { bubbles: true })); });
expect(mocks.invoke).toHaveBeenCalledWith("start_native_export", { terminal, session: attached.sessionId, token: exportToken });
expect(container.textContent).toContain("This file view expired");
```

运行 `pnpm -C ui exec vitest run src/sftp/M2C-export-stale.test.tsx --reporter=verbose`，实际 `1 passed`。

</details>


## 8. 2026-10-24 implementation response (not independent sign-off)

The original §7 findings, reproductions and source-line references above
remain the independent review's historical record. The implementer delivered:

| Finding | Response | Regression evidence |
| --- | --- | --- |
| 7.3.1 | Distinguish completed cleanup, verified owned residual, failed owned removal and unverifiable/replaced residual. Persist cleanup-only attributes/native identity or a non-executable inspection record. Reject resume/cleanup when authority is absent; preserve restart discovery and current-process visibility, and refresh after successful cleanup. | `failed_owned_upload_cleanup_keeps_a_baseline_bound_cleanup_record`, `failed_download_discard_keeps_an_identity_bound_cleanup_record`, augmented replaced-temporary regression, controller and bilingual queue tests |
| 7.3.2 | RAII reservation counts pending plus registered exports before stage creation; unregistered stages attempt bounded ownership-checked cleanup. | `concurrent_export_admission_never_leaves_an_orphan_stage`: nine concurrent requests admit eight; release leaves zero stages |
| 7.3.3 | Poll planning/preparing/ready/dragging states until terminal retained/failed; disable dragging when expired. | Ready → retained component regression checks disabled/non-draggable state, no drag command and stopped terminal polling |

[Final 18-check evidence](baselines/M2/raw/2026-10-23-closeout/engineering-checks.json),
[510-input source](baselines/M2/raw/2026-10-23-closeout/source-manifest-r3.json)
and [before/after identities](baselines/M2/raw/2026-10-23-closeout/source-changes-r3.json)
make this response reviewable without claiming a Git commit. Full production
SFTP tests include all 99 workflows. The SFTP v3 metadata-check/delete race
remains disclosed; a metadata baseline is not a filesystem transaction.
Independent verification of these fixes and affected desktop acceptance are
still pending. See [the current closeout](M2-CLOSEOUT.md).

## 9. 2026-10-25 independent follow-up

The maintainer explicitly authorized a read-only review subagent, separate
from the implementing parent. Its [source-bound report](baselines/M2/raw/2026-10-25-nonforeground-followup/INDEPENDENT-REVIEW.md)
preserves both passes: an initial persistence/restart cleanup-authority
finding and constructor rollback edge, followed by review of both fixes.
The r5 dependency patch was then independently checked without changing the
reviewed recovery/export implementation. No actionable scoped code blocker
remains identified. Execution and release readiness are recorded separately
in [the current follow-up](M2-FOLLOWUP.md); this does not sign off actual
desktop workflows, foreground performance, B8 or M2.
