# M2 集成审查与修复验收

> 2026-10-25 当前复核：维护者授权的独立只读子代理已复核三项原发现、两项补充故障分支及 r5 安全依赖更新，范围内无遗留代码阻断。具体源码身份、审查限制、工程执行和未签收门槛见 [本轮收尾](M2-FOLLOWUP.md) 与[独立报告](baselines/M2/raw/2026-10-25-nonforeground-followup/INDEPENDENT-REVIEW.md)。下方按日期保留原审查角色和结论。

> 2026-10-24 实施者收尾同步：M2-C 第 7 节三项发现已修复并补回归，18 项必需工程检查通过。修复详情、包身份和未签收项以 [当前收尾](M2-CLOSEOUT.md) 为准；以下原浏览审查及带日期续作不构成本轮独立复核。前台量测按维护者要求稍后继续，增量桌面和候选仍未签收。

> 日期：2026-10-17。角色：原浏览集成审查、修复与回归验证，后附本轮实现状态同步；不是新的独立 C 报告。  
> 当前结论（2026-10-18 维护者反馈及 A1 工程补齐后）：C1–C9 修复保留 accepted；可见单文件及独立 Explorer 清单已由维护者签收，A1 固定低/高 RTT 参考、预算和输入契约已补齐，A1–A6 及 A 门 accepted；B 门和 M2 blocked。M0/M1 保持 accepted。A1 属实施者自查和工程证据更新，不是新的独立 C 审查；下文旧时点的结论保留为历史。

## 单文件传输续作更新

2026-10-18 B 续作已接入固定库的 64 请求生产流水线、连续 ACK 进度和请求收敛，以及 OSC 7 parser/逐终端代次与序号校验、跟随和显式别名。生产单任务 1 GiB loopback/64 MiB 高 RTT 比较均满足原冻结数据、总耗时和绝对门槛；面板偏好原子持久化及退出收敛也已补齐；Rust 219/14/237、UI 146、vendor 检查及真实 OpenSSH 核心通过。目录跟随界面有新的范围内 `ship` 复核；人工增量、完整 B 功能/性能和候选仍未验收，见 [当前报告](M2-B.md) 与 [增量手测](M2-MANUAL.md)。Bash/Zsh 区块计划与 16 个真实 shell 场景也已通过，远程备份/预览/写入事务和安装入口仍缺。下方历史表格不作为本轮当前缺项清单；A 门签收不回退，也不扩大成 B 签收。

M2-05 已新增原生 picker grants、受控 Windows I/O、双向流式单文件传输、完整哈希/临时文件提交、冲突/取消、任务租约、3 worker 队列及退出协调。生产核心真实 WSL OpenSSH 上传/显式覆盖/下载/清理已通过；新增自动化与手测边界见 [M2-B](M2-B.md)，A 门逐项见 [M2-A](M2-A.md)。本节不把实施者自查写成独立安全审查，也不扩大原 C1–C9 的接受范围。下文的原审查与 162/14/176、UI 108 数字保留为浏览修复时点；当前总数和证据以 M2-B 为准。

维护者已回复“测试全部通过”；[来源与范围](baselines/M2/raw/2026-10-18-maintainer/README.md) 记录为汇总人工签收。此后 [A1 续作](baselines/M2/raw/2026-10-18-a1/README.md) 保留 16 请求失败、连接方法修正、正式低/高 RTT 参考、冻结防覆盖和完整回归，关闭 A 前置；未实现 B 工作与完整产品性能仍缺。已交付且未受影响的手测不重开；下文原表格和 `needs-maintainer-manual` 属历史，当前状态以 [M2-A](M2-A.md) 和 [board](board.md) 为准。没有新增独立 C 审查。

## 审查范围与差异依据

原集成审查对照任务、B/C 报告、契约、兼容表、PLAN 和当时源码，检查协议适配、连接复用、Rust 作用域、基础操作、Tauri 面板及原生拖出探针。该轮处理浏览实现缺陷；传输为其后的本轮新增实现，编辑/拖拽/续传等缺项继续保持 blocked，不能伪装成待手测项。

工作区没有 `.git` 元数据，无法提供提交间 Git diff。审查开始时保存了 161 个相关文件的完整副本到 `target/review-m2-2026-10-17-before/`，并归档 [修复前清单](baselines/M2/raw/2026-10-17-review/before-source-files.json) 和 [看板快照](baselines/M2/raw/2026-10-17-review/before-board.txt)。本轮源码/fixture 修改共 14 个文件（含新建集成测试），有可复核的 [文本 diff](baselines/M2/raw/2026-10-17-review/review-code.diff) 与 [前后哈希](baselines/M2/raw/2026-10-17-review/review-code-changes.json)。

[M1 选取文件比较](baselines/M2/raw/2026-10-17-review/m1-tracked-file-comparison.json) 显示其 287 项清单中 15 项现有文件哈希不同、0 项缺失。它只证明清单覆盖的差异；未列入 M1 清单的文件不能据此称为 M2 新增。最终 [源码与文档清单](baselines/M2/raw/2026-10-17-review/reviewed-files.json) 和 [证据索引](baselines/M2/raw/2026-10-17-review/manifest.json) 绑定本次交付，不虚构 commit 身份。

## 契约复核

架构分层符合当前已实现范围：`capstan-ssh` 负责协议和共享 subsystem，`capstan-core::sftp::SftpService` 负责终端绑定、浏览能力及操作，Tauri 仅做窗口校验/转发，前端通过 metadata DTO 与受控命令工作。正文与凭据不进入 WebView；未新增通用 shell/fs 能力；现有终端二进制 Channel/背压协议未被文件进度复用。本轮没有增加依赖或变更 PLAN。

新增进程内用例实际经过 `ConnectionPool`、`ConnectionLease::open_sftp` 和 `SftpService`，两个 shell 绑定的视图并发 attach 只认证一次、打开一个 SFTP subsystem。撤销过程中不再执行等待校验的 mutation，SSH 仍可打开 shell。这比原协议层探针覆盖更多生产接线，但尚未证明完整的 attach/失联/重连/空闲回收/锁库/退出状态矩阵，因此不能签收整个 A2。

`M2-CONTRACTS.md` 现已准确区分已实现服务、候选预算和未交付接口，记录 `not_found`、`omittedEntries`、事务导航、创建目录范围、单次确认、分页缓存及取消边界。固定版本库的存在和通过测试不等于 M2-01 的探针与数值冻结全部完成。

## C1–C9 修复签收

以下 `accepted` 限定对应修复和自动化证据，不代表完整 M2 或 Windows 交互签收。C 报告的原始描述与代码摘录保留为修复前证据。

| 编号 | 修复后的行为 | 验证 | 状态 |
| --- | --- | --- | --- |
| C1 空目录/分页 EOF | 过滤 dot 后继续填页，一项 lookahead 区分满页与真实后续页；空目录和精确 256 项末页均无多余下一页 | 真实协议集成覆盖空目录、首包 dot、精确末页及三页枚举；目录句柄回收 | `accepted` |
| C2 链接目录不能进入 | 双击/Enter 可对 directory 或 symlink 发起导航，服务端解析是否可进入 | symlink 导航/unlink 集成与面板键盘测试 | `accepted` |
| C3 失败导航破坏旧 scope | realpath、opendir、首屏读取和条目注册全部成功才替换 scope/cursor；失败保留旧条目与后续分页 | 不存在、opendir/readdir 无权限、重复名、dot 超限后的旧条目/确认与下一页均可用 | `accepted` |
| C4 不存在路径误报 | `ClientError::NotFound` 映射稳定 `not_found`，zh-CN/en 提供明确文案 | 集成错误映射、controller 失败导航与共享 fixture | `accepted` |
| C5 链接 chmod 无效确认 | prepare 前拒绝 symlink chmod，UI 禁用权限入口；仍允许链接删除和显式浏览 | 集成断言无远端 stat/修改，面板按钮状态 | `accepted` |
| C6 单个非法名阻断整个目录 | 跳过不可支持条目，累计 `omittedEntries`；合法文件继续可用，仅含非法名时显示告警而非空目录 | 非 UTF-8、非法组件、仅非法名目录；Rust/TS u32 解码与面板文案 | `accepted` |
| C7 create 父目录越过视图范围 | create 只允许当前成功浏览的规范目录，未浏览/其他/旧目录均在发送请求前拒绝 | 集成检查 mutation 计数与合法创建正例；这是应用能力边界，不是远端账户提权 | `accepted` |
| C8 已消费确认重复提交 | apply 成功或失败后均关闭弹窗，失败留在面板；再次尝试必须重新 prepare | 面板失败确认测试；服务端单次令牌回归 | `accepted` |
| C9 无上一页 | 增加上一页、下一页与页码；缓存最多 8 页，淘汰时释放 IDs，返回已淘汰页重新枚举 | 面板翻页、controller 缓存复用/释放/重新枚举 | `accepted` |

## 额外修复与保留边界

- 跨视图 apply 先验证 owner/session，再消费确认；错误调用不会销毁另一视图的合法 token。进程内用例覆盖跨视图拒绝后原所有者仍可执行，以及重复 apply 被拒绝。
- 每个 view 的目录/修改操作串行协调，等待后重验身份；Rust 导航代次与 UI 串行导航阻止旧结果覆盖新请求。如果旧导航已在后端提交而更新请求失败，UI 恢复实际已提交 scope。错误 session/页序清空不可信页并解除 busy。
- 终端与 view 的取消令牌贯通已 attach 的请求，撤销时不会在等待 lstat 后继续修改；scope 的有效期随授权访问更新。当前取消策略会关闭共享 SFTP subsystem，其他 SFTP 视图可能需要重新连接；没有宣称仅取消单个请求且其他 SFTP 请求不受影响。
- 关闭对话框后晚到的 prepare 不能重新打开它；dispose 后晚到的错误不能污染当前状态。修改已经成功但刷新失败时，保留刷新错误，不把修改判为失败诱导重复执行。
- 补充方向键、Home/End 行选择和焦点；文件名与手动路径保留首尾空格。全部新增用户文案具备 zh-CN/en；未扩大为界面重设计。
- 单次填页最多 64 次协议读取且总期限 15 秒，拒绝无限 dot/非法条目流。上限可能拒绝极度碎片化的服务器目录响应；这不是整次导航的硬实时保证。分页不是目录快照，返回已淘汰页时远端变化会反映到重新枚举结果。
- U+FFFD 来源无法区分时保守跳过，真实合法 U+FFFD 名也暂不支持；未声称实现原始字节文件名。远端属性比较不证明文件内容或身份相同，SFTP v3 的检查与修改之间仍存在服务器侧竞态，未承诺原子比较并修改。

## 原浏览审查的验证结果与实际覆盖

完整检查通过 `scripts/Check-M2.ps1` 的 rust/ui 两组执行，精确命令、退出码与原始日志见 [Rust manifest](baselines/M2/raw/2026-10-17-review/rust/checks-rust.json)、[UI manifest](baselines/M2/raw/2026-10-17-review/ui/checks-ui.json)。两组共 14 个命令退出码均为 0。

| 检查 | 结果 | 证据范围 |
| --- | --- | --- |
| fmt / 默认和全特性 Clippy | 通过，Clippy `-D warnings` | 当前 workspace |
| Rust 默认 / runtime / 全特性测试 | 报告 162 / 14 / 176 passed，0 failed | 含环境条件提前返回的测试；见下方 skip 说明，不称全部为实机执行 |
| 新增 core SFTP 集成 | 6/6，通过真实进程内 russh + 生产 pool/service | 运行时生成 key/password，不引入 Docker、静态私钥或新依赖 |
| Tauri runtime check | 通过 | 编译接线，不等于桌面流程验收 |
| 前端类型 / 单元测试 | 通过，17 文件 / 108 项；SFTP 24 项 | contracts 4、controller 12、panel 8 |
| 前端构建 | 通过 | Vite 对 742.85 kB minified JS chunk 保留 500 kB 阈值警告 |
| 既有布局 / 安全检查 | 28/28（scale 1、1.25）/ 10/10 | 布局脚本覆盖既有终端/选择器等页面，不覆盖 SFTP 可见工作流 |
| 依赖 / 许可 | 通过，394 Cargo、249 frontend 包 | cargo-deny 保留重复版本提示 |
| Impeccable 目标文件 detector | `[]`，无发现 | [输出](baselines/M2/raw/2026-10-17-review/impeccable-detect.json)，不代替视觉或原生交互手测 |

本轮完整 Rust 命令未设置 `CAPSTAN_TEST_SSHD`、`CAPSTAN_TEST_WSL`、`CAPSTAN_TEST_CONPTY_DIR` 和 `CAPSTAN_TEST_SFTP_PERFORMANCE=1`；对应外部 SSH/SFTP、WSL/捆绑 ConPTY 和性能探针明确输出 skip。保留先前 [OpenSSH r3](baselines/M2/raw/2026-10-17-openssh-r3/report.json) 的实际协议执行（OpenSSH 9.6p1、退出 0、fixture 清理成功），本轮没有复跑，也没有将其扩大为 core/桌面传输闭环证据。native staged-drag 两项既有测试同样是早先探针，原生代码本轮未修改。

已有 library performance 样本明确 `performanceAcceptance: false`；[policy](baselines/M2/performance-policy.json) 的 absolute/mixed-terminal 阈值仍为 null，不能拿样本或候选常量签收性能。未降低 80% 吞吐、1.5 倍文件树耗时或 M1 回归门槛。

## 未满足契约的阶段项

| 范围 | 状态 | 缺项与下一步 |
| --- | --- | --- |
| M2-01 / A1 | `blocked` | 完成扩展/提交失败语义探针、Windows 文件/目录实际拖出可行性、固定参考性能及预算冻结；staged data object 不等于可用拖拽下载 |
| M2-02 / A2 完整生命周期 | `blocked` | attach 中撤销、独立任务租约及真实 OpenSSH 核心已补；失联/重建、最后租约/空闲回收、锁定/退出与混合输入完整矩阵仍缺 |
| M2-03/04 已实现浏览及非递归操作 | `accepted`（原自动化子范围） | C1–C9 和新增边界回归保留；本轮新增 grant/提交单列，不扩大原接受范围 |
| 现有 Windows 浏览面板 | `needs-maintainer-manual` | 仅下节已实现交互，不以此代替 A6/B1 全部验收 |
| M2-05 / A4、A5、A6 | `needs-maintainer-manual`（已实现 picker/单文件子范围） | 受控双向传输/冲突/取消/哈希/故障保留已有自动化与真实生产核心证据；原生可见闭环仍缺，drop/export grants 未实现，A 门不签收 |
| B1 / M2-11 完整面板 | `blocked` | 停靠偏好持久化、目录跟随和完整工作流收口仍缺，基础面板不是完整 B1 |
| B2–B5 / M2-06–10 | `blocked` | 单文件队列已有；递归/双向拖拽、流水线/公平性、暂停/重试/双向 1 GiB 续传、编辑器保存与残留恢复、OSC 7/Bash/Zsh 仍缺 |
| B6 / M2-12 性能 | `blocked` | 无固定 WinSCP、受控高 RTT、混合终端负载、资源回收等验收数据 |
| B7 完整回归 / B8 候选 | `blocked` | 当前工程回归通过；完整 M2 实现、受影响桌面验收、v0.2 安装/便携/升级及最终签收仍缺 |
| M2 A 门 / B 门 / 总体 | `blocked` | 缺少实现和必需证据，不是单纯等待手测 |

后续先完成 M2-01/02 的剩余契约、前置验证和 M2-05 原生可见闭环；A1–A6 均 accepted 后再展开复杂工作流。新 [M2-A](M2-A.md) 如实记录 blocked，不冒充通过，也不生成或发布 v0.2 候选。

## 维护者手测清单（仅已实现范围）

状态为 `needs-maintainer-manual`，没有代填通过。准备隔离远端测试目录，记录当前源码/构建身份、Windows/WebView2、OpenSSH 版本、语言/主题/DPI及每项结果。下面只针对已实现的浏览和基础操作；完整阶段清单仍以 [M2 §8](M2.md#8-维护者手测验收清单) 为准。M0/M1 已接受且未受影响的手测不重开。

新增单文件选择器、传输/冲突、后台队列、关闭/锁定的可执行清单在 [M2-B](M2-B.md#maintainer-checklist-for-the-implemented-extension)，与下方原浏览清单合并使用。

- [ ] 同一主机两个 SSH 标签各浏览不同目录；快速切换、关闭其中一个后另一 shell 可用；本地 PowerShell/WSL 标签不出现 SFTP。
- [ ] 右/下停靠、折叠、调整尺寸、最小窗口、深浅主题及中英文可操作；首屏、加载、错误和省略条目提示不遮挡终端。
- [ ] 空目录、仅 dot 首包、256 项整页、多页目录、隐藏项与中文/空格名正常；上一页/下一页可用，跨 8 页返回时接受重新枚举。
- [ ] 手动路径、父目录、刷新和复制路径有效；不存在/无权限路径报错明确，失败后旧条目仍可操作与翻页。
- [ ] 在测试目录新建文件/目录、重命名、删除文件/空目录、chmod；确认展示正确主机/路径/类型，失败后不残留可重复提交的旧确认。
- [ ] 符号链接目录可双击/Enter 进入，链接权限按钮不可用；删除链接不删除目标；无效/非 UTF-8 名提示省略，合法文件仍可用。
- [ ] Tab/方向键/Home/End/Enter、对话框取消及焦点返回符合预期；取消后没有迟到确认重新出现。
- [ ] 请求期间关闭标签/视图或远端拒绝 SFTP 时不破坏其他 SSH shell；若共享 SFTP 因取消被关闭，其他面板能显示错误并通过显式重连恢复，不误称为 SSH 认证失败。

## 交付状态

最新结论写入 [board](board.md)，B/C 顶部已注明本轮修复与历史证据界限。未提交、推送、创建 tag 或发布；`docs/PLAN.md` SHA-256 保持 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`。
