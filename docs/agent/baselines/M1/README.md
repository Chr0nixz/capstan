# M1 验证证据

**2026-10-17 最终结论：M0、M1 A/B 门和 M1 总体 accepted。** 实现、工程结果、包身份、风险与未决问题见 [M1-B](../../M1-B.md)，阶段状态见 [board](../../board.md)。

| 最新证据 | 内容 |
| --- | --- |
| [最终 manifest](raw/2026-10-17-final/manifest.json) | 当前源码/锁文件与固定候选身份、验收结论与证据索引 |
| [工程检查](raw/2026-10-17-final-checks/checks.json) | Rust 120/14/134、UI 84、fmt/Clippy/审计/许可与实际捆绑 WSL/PowerShell 3 项 |
| [运行时](raw/2026-10-17-runtime/conpty-package.json) / [验证负例](raw/2026-10-17-runtime/validation-tests.json) | 官方包 hash、微软签名/PE/版本、三二进制布局与 MIT，5 场景验证 |
| [性能方法](PERFORMANCE.md) / [通过比较](raw/2026-10-17-visible-performance-r4/comparison.json) | 本机可见本地参考与三轮混合 20 标签、冻结阈值、字节/响应/资源；前三轮失败完整保留 |
| [OpenSSH Ed25519](raw/2026-10-17-final-sshd/report.json) / [RSA](raw/2026-10-17-final-rsa-host-sshd/report.json) | 各 12 认证/信任场景，无跳过，夹具清理 |
| [安装](raw/2026-10-17-final-install/report.json) / [便携](raw/2026-10-17-final-desktop/report.json) / [包内容](raw/2026-10-17-final-package/report.json) | 普通用户两版安装升级/卸载、数据保留、runtime 与许可、生产测量入口禁用 |
| [替换](raw/2026-10-17-final-replacement/report.json) | 原测试目录备份更新与启动，5 文件及标记 hash 不变 |
| [M0 实机确认](raw/2026-10-17-maintainer-m0/README.md) | 剩余设备范围按维护者明确报告 accepted；没有虚构逐项记录 |

下方保留 2026-10-16 的证据索引和当时状态；旧缺口由以上新实现与原始数据关闭，不改写旧报告。

---

最新自动化与交付收尾见 [M1-B](../../M1-B.md)，总体状态见 [board](../../board.md)。
维护者先报告 [P1/P2/P3 重点手测通过](raw/2026-10-16-maintainer-priority/README.md)，
随后报告 [剩余实机测试全部通过](raw/2026-10-16-maintainer-full-manual/README.md)，并明确
原始记录“未保存”。A7、跨机和其他实机功能范围不再待测；完整 M1 仍缺固定可见
性能原始证据与 M0 前置工程交付物。RSA 原依赖阻断已通过 AWS-LC 后端替换关闭。

| 2026-10-16 证据 | 内容 |
| --- | --- |
| [最新实机反馈](raw/2026-10-16-maintainer-full-manual/README.md) / [结构化状态](raw/2026-10-16-maintainer-full-manual/acceptance.json) | 全部实机测试按维护者报告通过，原始记录未保存；未补造数值或运行时材料 |
| [收尾 manifest](raw/2026-10-16-closeout/manifest.json) | 当时的自动化快照：候选/升级夹具 hash、源码和锁文件标识、检查汇总；其中旧待手测状态由最新反馈取代，文件不改写 |
| [workspace](raw/2026-10-16-closeout/workspace-tests.txt) / [runtime](raw/2026-10-16-closeout/rust-checks.txt) | 115 / 14 项；默认未配置的两条外部入口明确跳过，不计为真实服务器证据 |
| [cargo-deny](raw/2026-10-16-closeout/cargo-deny.txt) | 无 advisory 例外，四类检查均通过 |
| [Ed25519 主机密钥](raw/2026-10-16-closeout-sshd/report.json) / [RSA 主机密钥](raw/2026-10-16-closeout-rsa-host-sshd/report.json) | 真实 OpenSSH 各 12 场景，均无跳过 |
| [浏览器安全](raw/2026-10-16T13-17-15-155Z-terminal-security/report.json) / [布局](raw/2026-10-16T13-00-02-153Z-terminal-layout/report.json) | 10 / 24 项通过；安全测试隔离系统剪贴板 |
| [安装升级](raw/2026-10-16-closeout-install/report.json) | 普通用户 0.1.0→0.1.1 测试升级/卸载保留；当前账户，无干净 OS/新用户 VM 声明 |
| [便携 smoke](raw/2026-10-16-closeout-desktop/report.json) / [直接替换](raw/2026-10-16-closeout-replacement/report.json) | 启动/移动/受阻退出；原手测目录 exe 备份和数据哈希保留 |
| [原生终端全链路](raw/2026-10-16-closeout-terminal-pipeline/terminal-report.json) | debug、隐藏窗口五场景，62 PTY、186 输入探针、三轮各 20 会话；功能回归，不作可见/SSH 混合性能基线 |

`raw` 保留历史失败和成功，不覆盖原始报告。以相应日期、版本、明确范围和验证器
结论判断证据，不能因为有文件就记为通过。部分 `password.txt` 等名称是认证方式的
Cargo 测试输出日志，不是密码文件；真实秘密仅由一次性夹具在 `target` 下生成，
不复制到本目录。报告不收集终端正文、私钥、密码、主密码或真实 Vault。
