# M1 独立审查与错误路径/边界条件报告 (M1-C)

## 最终修复与验收更新（2026-10-17）

当前 M1 为 `accepted`。原发现 1～10 的修复回归继续通过；第 11 项受影响 RSA 依赖与 advisory 豁免已经移除。新补齐的 ConPTY 包/签名/companion、普通 WSL 入口、可见本地与 SSH 混合性能及安装交付均通过，维护者明确确认剩余 M0 实机范围也已通过。

最终工程结果为 workspace/runtime/全部特性 120/14/134 项、UI 84 项，fmt/Clippy/审计/许可通过；真实 OpenSSH 两种主机密钥各 12 场景、固定可见比较 28 项和候选包安装/便携核验均通过。原始数据、失败轮次和具体范围见 [M1-B](M1-B.md) 与 [最终 manifest](baselines/M1/raw/2026-10-17-final/manifest.json)。本段是修复和验证增量记录，不冒充一次新的独立代码/密码学审计。未修改 PLAN，未发布或提交。

以下保留原始独立审查与历次更新；旧的阻断/待测结论按当时日期解释。

> 日期：2026-10-12  
> 审查角色：C（独立审查与验证）  
> 依据：[`M1.md`](M1.md)、[`M1-B.md`](M1-B.md)、[`PLAN.md`](../PLAN.md)、[`VAULT-FORMAT.md`](../VAULT-FORMAT.md) 及当前源码  
> 审查范围：错误路径（Error Paths）、边界条件（Boundary Conditions）、回归问题（Regressions）及契约符合度  

2026-10-16 最新实机验收跟进：维护者在代码收尾后反馈“实机测试全部通过”，并说明原始报告/三轮性能数据与运行时包核验记录“未保存”。[反馈归档](baselines/M1/raw/2026-10-16-maintainer-full-manual/README.md) 将剩余 M1 功能手测记为维护者报告通过，A7 不再待测；完整阶段仍缺 §7.2 性能原始交付物和既有 M0 前置工程材料。此更新不产生新自动化结果或独立代码/密码学审查，后续各日期段落保留历史范围。

2026-10-16 自动化收尾跟进（实施者记录，不冒充新增独立审查）：缺陷 11 的受影响 `rsa` 依赖与审计例外已移除，RSA 运算改用固定 AWS-LC 后端；新生成的五种 RSA 文件格式在真实 OpenSSH 上通过，RSA 服务器主机密钥亦验证。生产 CSP、真实浏览器安全策略、安装/升级/卸载保留与交付文档已补齐。最新 workspace/runtime/UI 为 115/14/77 项通过，审计无 advisory 例外；实际证据和剩余实机范围见 [M1-B 收尾记录](M1-B.md#自动化与交付收尾2026-10-16)。下文缺陷列表及当时阻断保留历史；新密码学适配补丁的正常代码审核、原生安全交互、跨机及完整性能验收仍未签收。

2026-10-16 后续验收更新：维护者报告本轮 P1/P2/P3 全部通过，先前 P1 第 4 步失败项已关闭；Windows OpenSSH Agent 的重点 GUI 流程和换钥阻断/恢复均有维护者反馈。[验收记录](baselines/M1/raw/2026-10-16-maintainer-priority/README.md) 明确来源和范围；此前“待重点手测/待重试”按各节日期保留为历史。此更新不是新的独立代码审查，完整 A7 的剪贴板/粘贴、其余暂缓项目及既有发布阻断不在本次通过范围。

---

## 1. 审查概览与执行结论

本报告对 M1 阶段的代码实现进行了独立审查与测试复核。测试执行覆盖 Rust workspace、Tauri runtime 特性、前端类型检查、Vitest 单元测试、打包脚本、许可证合规与 M0/M1 测试工具链。

### 1.1 总体结论

1. **基本闭环与架构分层已建立**：`capstan-vault`（无 Tauri 依赖）、`capstan-ssh`（无 Tauri 依赖）、`capstan-core`（无 Tauri 依赖）、`capstan-terminal`、`src-tauri` 与 `ui` 的职责边界清晰，明文密码和私钥未进入 React 状态或 Tauri 通用事件，输出流继续保持 Raw 二进制 Channel 与背压流控，未引入通用 shell/fs 插件。
2. **核心自动化测试无回归**：2026-10-12 收口复验的默认 workspace 102 项、Tauri runtime 11 项、前端 58 项测试全部通过，Clippy 零警告，格式检查通过。本地 PTY 与共享终端契约未发生退化。
3. **发现 11 项边界条件、错误路径与契约缺陷**：
   - **缺陷 1（严重/DoS）**：Vault 目录项计数逻辑缺陷 — 数据目录下无关文件 >128 时导致所有写入事务及改密拒绝服务。
   - **缺陷 2（严重/恢复受阻）**：恢复候选扫描计数配额耗尽 — 8 个损坏或未完成写入的 `.pending` 临时文件会耗尽配额，阻止有效备份恢复。
   - **缺陷 3（契约未闭环）**：主机终端主题配置未生效 — 表单与 Vault 模型保存的 `theme`（Graphite / Light）在终端渲染层完全被忽略。
   - **缺陷 4（体验/诊断误导）**：前端 `showError` 未适配 `invalid_input` — 校验错误被统一误报为“连接失败”。
   - **缺陷 5（安全并发边界）**：移除主机信任后未递增 `vault_epoch` — 未能使持旧代次的在途建连尝试自动失效。
   - **缺陷 6（异常退出错误）**：网络断开后关闭终端标签抛出 `ssh_protocol` 异常。
   - **缺陷 7（资源拒绝服务）**：SSH 服务端未受限并发打开 Agent Forwarding 频道。
   - **缺陷 8（模型/CRUD 缺失）**：凭据管理缺少 CRUD 闭环，凭据无法编辑与删除。
   - **缺陷 9（网络边界过严）**：`normalize_target` 拒绝带下划线的内网主机名与带 Scope ID 的 IPv6 地址。
   - **缺陷 10（资源泄漏边界）**：已退出的远程 Shell 持续占用连接租约导致 30 秒空闲倒计时无法启动。
   - **缺陷 11（安全发布阻断）**：RUSTSEC-2023-0071（Marvin 时序攻击）未修复，维持阻断状态。

---

## 2. 工程检查与测试执行结果

在 Windows x64（Rust 1.96.0，Node 22.23.2，pnpm 10.28.1）环境下执行全量验证，实际结果如下：

| 检查项目 | 执行命令 | 实际结果 | 结论 |
| --- | --- | --- | --- |
| 代码格式 | `cargo fmt --all -- --check` | 退出码 0，无格式差异 | 通过 |
| 默认工作区测试 | `cargo test --workspace --offline` | 102 项全部 passed，0 failed | 通过 |
| Tauri Runtime 测试 | `cargo test -p capstan-app --features tauri-runtime` | 11 项全部 passed，0 failed | 通过 |
| 默认 Clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 退出码 0，零告警 | 通过 |
| Tauri Feature Clippy | `cargo clippy --workspace --all-targets --features tauri-runtime -- -D warnings` | 退出码 0，零告警 | 通过 |
| 前端类型检查 | `pnpm -C ui typecheck` | 退出码 0，零类型错误 | 通过 |
| 前端单元测试 | `pnpm -C ui test` | 12 个测试文件，58 项全部 passed | 通过 |
| 前端生产构建 | `pnpm -C ui build` | 构建通过（提示单个 chunk 超过 500 kB 警告） | 通过（有体积告警） |
| 许可证检查 | `powershell -ExecutionPolicy Bypass -File ./scripts/Check-Licenses.ps1` | 383 个 Cargo 包与 231 个前端包 SPDX 核验通过 | 通过 |
| M0 工具与语法 | `powershell -ExecutionPolicy Bypass -File ./scripts/Test-M0Tools.ps1` | 契约验证通过，4 报告拒绝 + 2 恶意 PE 分支通过 | 通过 |
| 依赖审计 | `cargo deny check all`（cargo-deny 0.20.2） | `advisories/bans/licenses/sources` 均报告 `ok`；重复 crate 版本产生提示；`RUSTSEC-2023-0071` 仍由 `deny.toml` 显式豁免 | 工具检查通过，RSA 风险仍阻断发布 |

---

## 3. 详细缺陷与边界问题清单

### 缺陷 1：Vault 目录下无关文件 >128 时导致所有写事务及改密拒绝服务 (DoS)

- **缺陷类别**：边界条件 / 错误路径 / 可用性破坏
- **代码位置**：`crates/capstan-vault/src/store.rs` 中的 `archive_previous()`、`prune_backups()`、`recovery_candidates()`
- **机理分析**：
  在 `store.rs` 中定义了 `const MAX_RECOVERY_FILES: usize = 128;`。
  在 `archive_previous()` 函数中：
  ```rust
  let mut highest = 0_u64;
  let mut inspected = 0;
  for entry in fs::read_dir(parent)? {
      let entry = entry?;
      inspected += 1;
      if inspected > MAX_RECOVERY_FILES {
          return Err(VaultError::InvalidData);
      }
      let name = entry.file_name().to_string_lossy().into_owned();
      let Some(suffix) = name.strip_prefix(&format!("{prefix}.")) else {
          continue;
      };
      ...
  ```
  `inspected` 计数器在判断文件名是否匹配 `prefix.` 之前就进行了自增。
  这意味着：只要 `parent`（即数据目录 `%APPDATA%\Capstan` 或便携目录 `./data`）下的**所有目录项总数**超过 128 个（无论这些文件是否属于 Capstan，例如日志、系统临时文件、测试夹具等），`inspected > 128` 就会立刻被触发，直接返回 `Err(VaultError::InvalidData)`。
  由于每次调用 `Vault::transaction()`、`Vault::change_master_password()` 以及发生迁移的 `Vault::unlock()` 时都会调用 `archive_previous()`，因此**一旦数据目录文件数达到 129，整个 Vault 的所有写入操作（新增/编辑主机、修改分组、修改偏好、改密）将全部被阻断**。
- **复现步骤与命令**：
  在临时测试中创建包含 129 个无关文件的目录并执行事务：
  ```rust
  let directory = tempfile::tempdir().unwrap();
  let path = directory.path().join("vault.capstan");
  let first = Vault::initialize(&path, &password(), fast()).unwrap();
  for index in 0..129 {
      fs::write(directory.path().join(format!("unrelated_{index}.txt")), b"dummy").unwrap();
  }
  let err = first.transaction(0, |data| Ok(())).unwrap_err();
  assert!(matches!(err, VaultError::InvalidData));
  ```
- **实际结果**：
  返回 `VaultError::InvalidData`。在前端表现为报错，配置无法保存。
- **修复建议**：
  应先检查文件名是否以 `format!("{prefix}.")` 开头，仅对属于本 Vault 的候选文件进行计数限制；或者即使目录项过多，也不应导致主文件的正常写入事务失败。

---

### 缺陷 2：8 个无效或 0 字节的 `.pending` 临时文件会耗尽配额，阻止有效备份恢复

- **缺陷类别**：边界条件 / 错误路径 / 恢复路径受阻
- **代码位置**：`crates/capstan-vault/src/store.rs` 中的 `recovery_candidates()` (lines 249-268)
- **机理分析**：
  `candidate_attempts` 的初衷是限制高开销的 Argon2id 密码派生次数（上限 8 次）。
  但在 `recovery_candidates()` 实现中：
  ```rust
  let is_pending = suffix.ends_with(".pending");
  let is_backup = suffix.starts_with("backup-") && suffix.ends_with(".cav");
  if !is_pending && !is_backup {
      continue;
  }
  if candidate_attempts >= MAX_RECOVERY_CANDIDATES {
      return Err(VaultError::RecoveryLimit);
  }
  candidate_attempts += 1;
  if !entry.file_type()?.is_file() {
      continue;
  }
  let candidate_bytes = match read_limited(&entry.path()) {
      Ok(bytes) => bytes,
      Err(VaultError::InvalidFormat) => continue,
      Err(error) => return Err(error),
  };
  let Ok(opened) = envelope::open(candidate_bytes, password, EnvelopeKind::Vault) else {
      continue;
  };
  ```
  `candidate_attempts += 1` 放置在检查 `is_file()`、检查文件大小、读取文件内容之前。
  如果由于断电、崩溃或并发写中断在磁盘上残留了 8 个 0 字节的 `.pending` 临时文件，它们并不执行 Argon2id 派生（在 `read_limited` 时即因格式不符 `continue`），但已经将 `candidate_attempts` 累加到了 8。
  当处理到后面的有效备份文件 `backup-xxx.cav` 时，函数直接报错 `VaultError::RecoveryLimit`，导致用户完全无法列出或选择有效备份进行恢复。
- **复现步骤**：
  在 Vault 目录下创建 1 个有效备份文件和 8 个 0 字节的 `vault.capstan.junk-0..7.pending` 文件，调用 `Vault::recovery_candidates`。
- **实际结果**：
  返回 `Err(VaultError::RecoveryLimit)`，扫描直接失败，有效备份被屏蔽。
- **修复建议**：
  仅在文件通过大小校验并真正准备调用 `envelope::open` 进行 KDF 派生前，才递增派生计数器。

---

### 缺陷 3：主机配置中的终端主题（`theme: "graphite" | "light"`）未在前端生效

- **缺陷类别**：功能缺失 / 契约未闭环
- **代码位置**：`ui/src/terminal/TerminalPane.tsx`、`ui/src/terminal/session.ts`、`ui/src/terminal/contracts.ts`
- **机理分析**：
  M1.md §6.3 明确规定：“字段必须有实际行为与测试，不能只提供无法生效的表单。……默认目录、启动命令、声明支持的编码和终端配色实际生效”。
  在后端 `crates/capstan-vault/src/model.rs` 中，严格限定了 `theme` 只能为 `"graphite"` 或 `"light"`；UI 的主机编辑器也提供了相应选项并持久化存储。
  然而在前端运行时链路中：
  1. `TerminalLaunch` 类型仅有 `{ kind: "ssh"; host_id: HostId; attempt_id: string }`，未包含 `theme`。
  2. `TerminalPane.tsx` 与 `session.ts` 中完全没有读取或设置 xterm 主题的逻辑（检索 `theme` 关键字仅在 `contracts.ts` 中有一处属性定义）。
  3. xterm 实例在初始化时始终使用默认配色，用户选择的 Light 或 Graphite 主题从未生效。
- **复现步骤**：
  配置一个主机并将主题设置为 `light`，保存并点击连接。
- **实际结果**：
  终端依然渲染为默认深色背景，主题设置没有任何视觉或状态效果。

---

### 缺陷 4：前端 `showError` 未适配 `invalid_input`，校验错误被误报为“连接失败”

- **缺陷类别**：错误路径 / 用户体验与诊断
- **代码位置**：`ui/src/App.tsx` 中的 `showError` (lines 173-190)
- **机理分析**：
  Rust 后端在输入参数不符合要求（如端口越界、分组循环依赖、标签超标、不可达的主机地址格式）时，返回的标准错误码为 `"invalid_input"`。
  但在前端 `showError` 中：
  ```ts
  const showError = (cause: unknown): void => {
    const code = errorCode(cause);
    if (code === "host_key_changed") setError(labels.hostKeyChanged);
    ...
    else if (code === "vault_data") setError(labels.invalidVaultData);
    else setError(labels.connectionFailed);
  };
  ```
  缺少针对 `code === "invalid_input"` 的显式分支，因此所有这类校验错误全部掉入 `else` 分支，被报告为 `labels.connectionFailed`（中文："连接失败"，英文："Connection failed"）。
- **复现步骤**：
  在主机管理中构造一个后端拒绝的非法输入，或在分组管理中将父分组拖入子分组触发循环依赖拒绝。
- **实际结果**：
  界面顶部提示“连接失败”，提示内容与实际发生的业务错误严重不符。

---

### 缺陷 5：移除主机信任（`forget_trusted_host_key`）后未递增 `vault_epoch`

- **缺陷类别**：安全状态与并发边界
- **代码位置**：`crates/capstan-core/src/lib.rs` 中的 `forget_trusted_host_key` (lines 717-757)
- **机理分析**：
  M1.md §4.3 规定：“信任修改与导入冲突都会失效相关的待确认操作和后续复用资格。”
  当导入备份（`import_backup`）时，代码执行了 `self.epoch.fetch_add(1, ...)`，使所有持旧 `vault_epoch` 的建连尝试在 `is_current()` 检查中判定失效。
  但在 `forget_trusted_host_key` 中，执行事务后并未更新 `self.epoch`。这导致若用户在外部发现主机密钥可疑并手动移除了该信任记录，同时恰好有在途建连尝试持有该代次，该尝试不会因为信任被遗忘而自动失效。
- **实际影响**：
  无法保证在途尝试立即因信任策略变更而阻断。

---

### 缺陷 6：远程连接断开后关闭终端标签抛出 `ssh_protocol` 异常

- **代码位置**：`crates/capstan-ssh/src/lib.rs` 中的 `SshChannelControl::close()` (lines 651-655) 与 `capstan-terminal` 交互
- **机理分析**：
  ```rust
  pub async fn close(&self) -> Result<()> {
      self.writer.eof().await.map_err(|_| SshError::Protocol)?;
      self.writer.close().await.map_err(|_| SshError::Protocol)?;
      self.exit.send_replace(ChannelExit::Ended(None));
      Ok(())
  }
  ```
  当远程主机网络中断、TCP 异常断开或进程崩溃时，底层的 SSH 传输已经不存在。
  用户随后点击标签栏的“关闭”按钮时，`close()` 仍然尝试发送 `writer.eof()` 和 `writer.close()`，此时底层信道报错，导致 `close()` 返回 `Err(SshError::Protocol)`，后续的 `self.exit.send_replace(ChannelExit::Ended(None))` 无法执行。
  终端 supervisor 将此关闭操作捕获为后端错误，使得一个已经死亡会话的正常关闭操作在后台产生异常。

---

### 缺陷 7：SSH 服务端未受限并发打开 Agent Forwarding 频道

- **代码位置**：`crates/capstan-ssh/src/lib.rs` 中的 `server_channel_open_agent_forward` (lines 460-468)
- **机理分析**：
  ```rust
  async fn server_channel_open_agent_forward(
      &mut self,
      channel: russh::Channel<client::Msg>,
      _session: &mut client::Session,
  ) -> std::result::Result<(), Self::Error> {
      let connector = self.agent_forwarding.clone();
      tokio::spawn(relay_agent_channel(channel, connector));
      Ok(())
  }
  ```
  当主机启用了 Agent forwarding 后，服务端每请求打开一次 agent-forward 通道，客户端无条件 `tokio::spawn` 一个任务并通过具名管道连接本机 Agent。
  缺少对当前活动转发通道数量的上限控制（Bounded Semaphore），恶意的远程服务端可通过高频请求并发打开大量通道耗尽本机资源。

---

### 缺陷 8：凭据管理缺少 CRUD 闭环，凭据无法编辑与删除

- **代码位置**：`crates/capstan-core/src/lib.rs`、`src-tauri/src/lib.rs`、`ui/src/App.tsx`
- **机理分析**：
  目前凭据仅实现了添加接口（`add_password_credential` 等）与下拉列表展示，没有提供 `delete_credential` 或 `update_credential`。
  用户录入错误密码或需要更换私钥时，无法删除无用凭据，只能不断创建新凭据。M1.md §6.3 要求的“删除主机/凭据须检查引用并确认影响”未在凭据上完整实现。

---

### 缺陷 9：`normalize_target` 拒绝带下划线的内网主机名与带 Scope ID 的 IPv6 地址

- **缺陷类别**：边界条件 / 目标地址规范化过度限制
- **代码位置**：`crates/capstan-vault/src/model.rs` 中的 `normalize_target` (lines 181-208)
- **机理分析**：
  在 `normalize_target` 中：
  ```rust
  let host = value.trim_end_matches('.').to_ascii_lowercase();
  if !host.is_ascii()
      || host.split('.').any(|part| {
          part.is_empty()
              || part.len() > 63
              || part.starts_with('-')
              || part.ends_with('-')
              || !part
                  .bytes()
                  .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
      })
  {
      return Err(VaultError::InvalidData);
  }
  ```
  1. RFC 1123 规范中标准 DNS 主机名只允许字母、数字和连字符 `-`。但现代实际工程环境（如 Docker Compose 容器名 `dev_db_1`、Kubernetes 命名、mDNS 或内网自定义域名 `bastion_proxy.local`）普遍存在下划线 `_`。当前实现因为 `!part.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')` 会直接返回 `VaultError::InvalidData`。
  2. 链路本地（Link-Local）IPv6 地址常带有网络接口标识符（Scope ID / Zone Index），如 `[fe80::1%eth0]` 或 `[fe80::1%12]`。由于 Rust `std::net::IpAddr::from_str` 不支持解析带 `%` 的 Scope ID，`ip_value.parse::<std::net::IpAddr>()` 失败后回退到域名解析，域名解析又因为 `%` 和 `:` 失败，最终返回 `VaultError::InvalidData`。
- **复现步骤**：
  ```rust
  assert!(normalize_target("dev_web_1.local").is_ok()); // 失败，返回 Err(InvalidData)
  assert!(normalize_target("[fe80::1%eth0]").is_ok());  // 失败，返回 Err(InvalidData)
  ```
- **实际结果**：
  合法内网容器名或链路本地 IPv6 地址无法被保存为主机。
- **修复建议**：
  在域名部分放宽支持下划线 `_`（非开头/结尾）；对 IPv6 地址剥离并保留可选的 `%<scope>` 后缀进行规范化。

---

### 缺陷 10：已退出的远程 Shell 持续占用连接租约导致 30 秒空闲倒计时无法启动

- **缺陷类别**：生命周期与资源回收边界 / 连接池泄漏
- **代码位置**：`crates/capstan-ssh/src/lib.rs` 中的 `SshChannelControl` (lines 628-634) 与 `crates/capstan-terminal/src/service.rs`
- **机理分析**：
  M1.md §4.4 规定：“最后一个通道释放后保留 30 秒；新租约取消空闲关闭，超时才关闭。”
  当远程 shell 自然退出（例如在终端内键入 `exit` 或远程进程被杀死）时：
  1. russh 通道进入 EOF / Close，reader task 退出并将状态标记为 `ChannelExit::Ended`。
  2. 共享终端的 supervisor 任务将 `TerminalSnapshot.state` 置为 `Exited`。
  3. 但前端为了保持已退出标签的历史输出回滚，标签并未关闭，前端也未调用 `stop_terminal`。
  4. `TerminalService` 中的 `TerminalSession` 实例依然存活，持有 `control: ControlEndpoint::Async(Arc<dyn AsyncBackendControl>)`。
  5. 关键问题在于：`SshChannelControl` 中持有的租约 `_lease: Option<ConnectionLease<PasswordConnection>>` 是不可变字段，未封装在 `Mutex` 中，在通道退出或 `close()` 时**无法主动释放**。
  6. 结果是：只有当用户在 UI 上显式点击该标签的“×”（关闭标签）触发 `stop_terminal` 销毁 `TerminalSession` 时，`ConnectionLease` 才会被 drop 并扣减 `leases` 计数。在此之前，即使远端通道早已死亡，底层连接池的 `leases` 始终为 1，30 秒空闲计时器无法启动，底层 TCP 保持长期连接（直到达到 russh 的 60s inactivity timeout）。
- **复现步骤**：
  1. 通过 Capstan 连接一个 SSH 主机，成功打开一个标签。
  2. 在终端内键入 `exit` 并回车，终端提示 `Exited with code 0`。
  3. 保持该标签打开，等待 40 秒。
  4. 检查连接池状态。
- **实际结果**：
  由于该死标签仍处于打开状态，`ConnectionLease` 未被 drop，连接池中依然保留着该连接；直到点击关闭标签，租约才被释放。
- **修复建议**：
  将 `SshChannelControl` 中的 `_lease` 包装为 `Mutex<Option<ConnectionLease<PasswordConnection>>>`，在通道收到 EOF/Exit 或者 supervisor 判定退出后主动 `.take()` 释放租约。

---

### 缺陷 11：安全阻断项维持 — RUSTSEC-2023-0071（Marvin 时序攻击）

- **代码位置**：`deny.toml`
- **机理分析**：
  依赖项 `russh` 的 RSA 路径依赖 `rsa 0.10.0-rc.18`，存在未修复的 Marvin timing attack 漏洞。
  `deny.toml` 中通过显式配置例外绕过了检查。在替换或修复 RSA 实现前，该项维持为**安全发布阻断项**，不具备发布 v0.1 的条件。

---

## 4. 回归检查结论

对现有既有功能的回归测试检查结论如下：

1. **终端二进制 Channel 与流控契约**：未发生退化。大帧 Raw 二进制 fetch 路径、小帧 1024 字节补零强制走二进制路径、背压机制（>1 MiB 暂停，<256 KiB 恢复）、ACK 机制及 100 MiB 流量校验完整保留。
2. **本地 PTY 探测与会话**：PowerShell 探测、WSL 解析、进程退出码捕获及关闭等待测试全部通过。
3. **前端生命周期**：标签切换、隐藏释放 WebGL、搜索及快捷键、OSC 52 权限控制、多行粘贴拦截全部通过。
4. **便携模式与数据目录判定**：`capstan.portable` 标记识别、数据目录探针读写验证及只读失败回退逻辑测试通过。

---

## 5. M1 验收门符合度独立评估

对照 `docs/agent/M1.md` 第 5 节规定的两道验收门逐项评估：

### 5.1 A 门：Vault 与单条 SSH shell

| 场景 | 要求 | 自动化证据 | 实机/真实环境状态 | 独立审查结论 |
| --- | --- | --- | --- | --- |
| **A1 新建与解锁** | 正确/错误密码、锁定/解锁、非法 KDF/版本/损坏不覆盖 | 当前 `capstan-vault` 20 项测试通过 | 需实机手测原生窗口交互 | 自动化符合契约；此前目录项计数 DoS 缺陷已按缺陷 1 回归收口 |
| **A2 最小持久化** | 原生录入密码凭据，保存主机，重启恢复，明文不入 Web | 当前 `capstan-core` 14 项测试通过 | 需实机检查 WebView 状态/日志 | 符合自动化范围，秘密字段脱敏严格 |
| **A3 信任先于认证** | 未知密钥确认保存，拒绝/取消不认证，变化阻断，重启免确认 | 当前 `capstan-ssh` 19 项单元测试覆盖进程内验证 | 需真实 sshd 手测弹窗对比 | 符合自动化范围，密钥变化弹窗只读阻断有效 |
| **A4 密码认证与 shell** | 正确/错误密码，PTY/shell 请求，输入/resize/正常退出 | 进程内服务端测试通过 | 缺少默认 WSL OpenSSH 真实链路 | 自动化符合契约；真实 OpenSSH 测试已留 harness 入口 |
| **A5 共享终端契约** | SSH 异步适配现有预算/背压/ACK，100 MiB 无乱序 | `capstan-terminal` 20 测试全部通过 | 隐藏 WebView2 测量通过，可见测量待补 | 共享协议无退化 |
| **A6 写入与异常恢复** | 故障注入，原子替换，坏候选不覆盖原库 | 分阶段写入故障注入与恢复扫描配额测试通过 | 真实磁盘掉电未测 | 符合自动化范围；此前无效 pending 文件耗尽配额问题已按缺陷 2 回归收口 |
| **A7 真实闭环** | Windows/WebView2 连接指定 WSL OpenSSH | 真实 sshd harness 已提供（默认跳过） | 目标环境未配置，未实测 | `needs-maintainer-manual` |

### 5.2 B 门：日常使用与交付

| 场景 | 要求 | 自动化证据 | 独立审查结论 |
| --- | --- | --- | --- |
| **B1 主机工作流** | CRUD、分组、拖放、收藏、搜索、重启保持 | 自动化测试覆盖核心逻辑及凭据更新/删除引用检查 | 真实桌面持久化与确认流程待手测 |
| **B2 三类认证** | 密码、私钥（加密/非加密/路径引用）、Agent/Pageant | Ed25519 运行时密钥测试通过 | 真实 Windows Agent/Pageant 待手测 |
| **B3 连接池** | 同配置并发单次认证，隔离，30s 保留，慢通道隔离 | 进程内双通道压测及自然退出租约释放回归通过 | 真实服务器连接生命周期待手测 |
| **B4 日常终端** | 混合标签、搜索、隐藏释放 WebGL、断线终态 | 前端 58 项测试通过 | 主机 theme 已传入 xterm；真实桌面主题与交互待手测 |
| **B5 Vault 生命周期** | 改主密码、自动锁定、断开选项、3 份有效备份 | 自动化测试全部通过 | 核心功能完整 |
| **B6 跨机迁移** | 独立口令导出 `.cbak`，合并/覆盖预览，信任冲突阻断 | 自动化测试覆盖预览与合并 | 符合契约，信任冲突强制阻断有效 |
| **B7 安装与便携** | NSIS 安装、升级、卸载数据保留，便携标记识别 | 打包脚本与目录探测测试通过 | 仅产出无签名测试包，未在隔离用户验收 |
| **B8 回归与验收** | 全部工程检查通过，阻断项关闭 | 格式/Clippy/测试/许可全部通过 | **RUSTSEC-2023-0071 阻断 v0.1 发布** |

---

## 6. 验收建议与后续步骤

缺陷 1–10 的修复已经落入当前源码并通过回归，见本报告第 7 节。当前后续事项如下：

1. **已完成：优先修复严重缺陷**：
   - 修复 `crates/capstan-vault/src/store.rs` 中的目录项遍历计数逻辑，确保无关文件不触发 `InvalidData`。
   - 修复 `recovery_candidates` 中派生计数器位置，仅对实际尝试 KDF 派生的候选进行配额扣减。
   - 修复 `SshChannelControl` 租约生命周期，在远端通道退出后主动释放租约，启动 30 秒倒计时。
2. **已完成：补齐前端体验与状态细节**：
   - 在前端终端创建链路中接入并应用 `host.theme`。
   - 在 `App.tsx` 的 `showError` 中增加 `invalid_input` 错误码的映射，展示准确的错误信息。
   - 在 `forget_trusted_host_key` 成功后递增 `vault_epoch`。
   - 放宽 `normalize_target`，支持合法带下划线的主机名与链路本地 IPv6。
3. **已完成：补充凭据管理 CRUD 闭环**：
   - 补充凭据删除命令与引用安全检查。
4. **待维护者实测**：
   - 维护者按 M1-B 清单验证真实 Windows OpenSSH Agent 转发、WSL OpenSSH 闭环及便携包实机体验。
5. **仍为发布阻断**：
   - 定位并替换/修复 `russh` 下层 `rsa 0.10.0-rc.18` 的 Marvin 时序攻击漏洞，移除 `deny.toml` 临时豁免。

## 7. 2026-10-12 收口复验与缺陷状态

当前复验执行 `cargo test --workspace --offline` 共 102 项、Tauri runtime 11 项和 UI 58 项，均通过。M1-C 缺陷 1–10 的修复在当前源码中仍可见，回归覆盖包括：`unrelated_directory_entries_do_not_block_writes_rotation_or_password_changes`、`invalid_pending_files_do_not_spend_the_recovery_kdf_quota`、`trusted_host_key_management_redacts_public_key_and_uses_revisioned_forget`、`closing_after_transport_disconnect_is_idempotent`、`natural_shell_exit_releases_pool_lease`、`targets_preserve_ipv6_scope_and_normalize_internal_host_names`，以及对应主题、`invalid_input`、转发并发上限和凭据 CRUD 的 UI/core/SSH 测试。


RSA 风险复核结果没有变化：RustSec advisory 数据仍声明 `patched = []`。`rsa 0.10.0-rc.19` 于 2026-10-06 发布，关联提交只调整对 `pkcs1` 版本的兼容；跟踪的隐式拒绝与默认 blinding PR 当日仍未合并。因此保留显式 advisory 例外作为待解决项，不能将 `cargo deny check all` 的成功解释为 RSA 风险已消失。M1-SSH 兼容矩阵继续限制 RSA 发布结论。

真实 sshd 用例在本机输出明确 `SKIP`，原因是 `CAPSTAN_TEST_SSHD` 未设置；A7、Windows OpenSSH Agent/Pageant、桌面安装/便携运行和性能验收仍需实际设备证据。A 门实施记录见 [`M1-A.md`](M1-A.md)，本复验不将自动化覆盖冒充真实环境验收。

## 8. 2026-10-13 后续实施与实际联测备注

本节记录后续实施结果，不将其标为一次新的独立审查。真实 WSL OpenSSH 9.6p1 / Windows Pageant 0.85 联测发现：开启 Agent forwarding 后第二个 shell 因重复请求被服务器拒绝。该兼容性缺陷已修复为同一传输上异步串行确认一次请求；进程内服务端拒绝重复请求，并发双 shell 验证只有一次请求和一次认证。真实 Pageant 转发关闭→开启→再次关闭、双 shell、远端签名以及实际换钥阻断均通过。

真实测试原始报告、KDF 参数测量、104 项 workspace / 11 项 runtime / 58 项 UI 回归与遗留临时凭据目录的清理限制见 [`M1-B.md`](M1-B.md) 最新节。Windows OpenSSH Agent 服务未启用，桌面 A7 和完整固定性能/安装验收仍未签收。RSA advisory 2026-10-13 查询仍无 patched versions，安全发布继续阻断；`PLAN.md` 未修改。

实际 Windows 启动还发现第二次 Tauri `Builder::setup` 覆盖状态初始化的致命错误，导致 `state() called before manage()` / exit 101。已将 Mica 与基准窗口逻辑并入唯一 setup；新增 `Test-M1Desktop.ps1` 用旧包重现失败，新包通过初次启动、移动后启动和阻塞数据路径非零退出三个场景。该回归不包含解锁/渲染交互；用户停止 computer-use 后未继续界面输入，也未把不一致截图作为通过证据。基准 feature 的一处 Clippy 提示已修正，12 项基准/runtime 测试与 Clippy 通过。

## 9. 2026-10-13 桌面反馈：终端高度回归

后续 P1 修复与最新包以第 10 节及 M1-B 最新节为准。

维护者实际截图显示终端只占一行。常规 `.workspace` 已从旧的标题栏/标签栏/终端三项改为标签栏/终端两项，CSS 却仍有三行，导致终端固定在 34px 的第二行。组件单元测试的 jsdom 与 TerminalPane mock 未覆盖真实布局，是既有验收缺口。已为常规/基准工作区分别定义两行/三行，明确剩余轨道和单列最小尺寸。

新增 `pnpm -C ui test:layout` 驱动生产 CSS 与真实 xterm：24 个尺寸/标签/搜索/缩放场景通过，旧 desktop 场景 34px shell 已恢复到 1600×1000 窗口下的 930px。原始失败/通过数据与当前包见 [`M1-B.md`](M1-B.md) 最新节。此处是后续实现与验证补充，不作为新的独立审查，不以浏览器布局检查代替原生认证或完整 A7。PLAN 未修改。

## 10. 2026-10-16 P1 信任确认与启动错误复验

维护者的“信任并保存后无法启动本地 Shell”反馈暴露了自动化缺口：所有旧 SSH 测试收到指纹后立即确认，未覆盖人工核验耗时；前端启动错误测试也未覆盖 SSH 失败被压成 `backend` 后回落到本地 Shell 文案。

现已用进程内服务复现等待 20 秒导致 15 秒网络握手超时，修复为主机密钥核验阶段暂停网络预算，独立保留 120 秒确认期限。过期和丢弃启动任务都会清理待确认请求，认证前零请求、迟到确认不保存、确认后剩余网络预算继续受限均有回归。固定公开错误码通过共享终端和 Tauri 适配器保留；任意动态错误详情仍被脱敏，SSH 未知错误也不会显示本地 Shell 文案。

新 Core 集成测试用独立临时 Vault 覆盖存储凭据、拒绝/保存信任、错误密码、凭据更新和重新打开 Vault 后连接。对维护者当前专用 WSL OpenSSH 的定向运行实际等待 20 秒再确认、打开 shell、重新打开临时 Vault 后直接连接，2 项通过。没有读取维护者实际 Vault 或改变其 sshd。全量 workspace 112 项（默认两项外部入口按配置跳过）、Tauri runtime 13 项、UI 77 项及格式、两组 Clippy、类型检查/构建通过。

这是后续实现复验，不是新的独立审查；旧桌面尝试的原始错误码未留存，仍需维护者用新包重试 P1 第 4 步，不把后端通过记为 A7 完整签收。包、替换记录和剩余边界见 [M1-B 最新节](M1-B.md)。PLAN 未修改。
