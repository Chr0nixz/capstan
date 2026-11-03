# Contributing / 参与开发

Read [AGENTS.md](AGENTS.md) and [docs/PLAN.md](docs/PLAN.md) before changing the
architecture. Keep domain logic in `crates/`; Tauri adapters translate commands
and events. Do not enable general-purpose shell/filesystem plugins. Terminal
output uses raw binary Channels with bounded queues and ACK-based backpressure.

Install the tools listed in [README.md](README.md), then run:

```powershell
pnpm -C ui install --frozen-lockfile
powershell -NoProfile -File scripts/Prepare-ConPty.ps1
powershell -NoProfile -File scripts/Test-ConPty.ps1
cargo fmt --all -- --check
cargo fmt --manifest-path vendor/ssh-key/Cargo.toml -- --check
cargo test --workspace --offline
cargo test -p capstan-app --features tauri-runtime --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo clippy --workspace --all-targets --features tauri-runtime --offline -- -D warnings
pnpm -C ui typecheck
pnpm -C ui test
pnpm -C ui test:security
pnpm -C ui test:layout
pnpm -C ui build
cargo deny check all
powershell -NoProfile -File scripts/Check-Licenses.ps1
```

An offline run validates cached dependencies; it does not update the advisory
database. Run `cargo deny` with network access for a release check. CI is defined
in [.github/workflows/windows.yml](.github/workflows/windows.yml); a checked-in
workflow is not evidence of a successful remote run.

Generate test keys at runtime. Use the in-process russh server for integration
tests; optional real OpenSSH checks and their disposable fixture are described
in [M1-SSH-COMPAT.md](docs/agent/M1-SSH-COMPAT.md). Never use production keys or
hosts for rotation, forwarding, damaged-Vault or import-conflict tests.

Preserve the license/source notices for patched dependencies. The RSA backend
patch is documented in [vendor/ssh-key/CAPSTAN_PATCH.md](vendor/ssh-key/CAPSTAN_PATCH.md).
Use English identifiers and comments, bilingual user-facing copy, focused fixes
with regression coverage and Conventional Commit messages. Do not commit or
push on behalf of a maintainer without an explicit request.

## 中文

开发前阅读 AGENTS.md 和 PLAN.md。业务逻辑放在独立 crate，Tauri 只做接线；保持原始
二进制终端通道、ACK/背压、凭据边界、主机换钥阻断和最小权限。修改方案需先由维护者
确认，不能把缺失的环境或跳过的测试记为通过。

按 README 安装工具，交付前执行上方检查。离线测试不代表漏洞库已更新；真实服务器
测试使用一次性账户和现场生成的密钥。日志、报告和 PR 中不得出现密码、私钥、口令、
真实 Vault 或终端正文。保留补丁依赖的来源与许可证；代码、注释使用英文，用户文字
提供中英翻译。未经要求不要提交或推送。
