# Security / 安全

Capstan is under development. Full release acceptance and supported scenarios
are recorded in [the review board](docs/agent/board.md) and
[the compatibility matrix](docs/agent/M1-SSH-COMPAT.md).

Credentials stay in the desktop Rust process. Local Vault and backup encryption
protect stored data; they do not make an unlocked, compromised Windows session
safe. Keep the master password and independent backup password available through
your own secure recovery process. Capstan has no password-reset service.

Verify first-use host keys independently. Changed keys block authentication.
Agent forwarding is off by default: enabling it authorizes the remote host to
request signatures from the selected Agent service. Existing forwarded sessions
must be closed to revoke their access.

RSA-SHA2 signing uses AWS-LC through a documented ssh-key patch. The former
RustCrypto `rsa` dependency and its RUSTSEC-2023-0071 exception have been removed.
This is an implementation replacement, not a new cryptographic audit.

There is currently no dedicated private vulnerability-reporting channel listed
in this checkout. Contact the maintainer through a private channel you already
trust before disclosing exploit details. Do not put sensitive reports into a
public issue. Share only affected versions, stable error codes and sanitized
reproduction steps; generate disposable test material for reproductions.

Never attach passwords, private keys, passphrases, real Vault/backup files or
terminal transcripts. Unsigned candidate artifacts are for controlled testing;
verify their SHA-256 against the recorded build result.

In-app updates require an embedded trusted public key, an artifact signature and
a matching signed version. Missing keys disable updates; release metadata cannot
replace the trusted key. Production signing secrets belong only to the protected
`release` GitHub Environment. Authenticode signing is separate and is not provided.
See [update security and recovery](docs/UPDATES.md).

## 中文

项目仍在开发，支持范围和完整验收状态见 board 与兼容矩阵。密码和私钥仅在 Rust
侧处理；加密库保护磁盘数据，解锁后的 Windows 会话仍需由操作系统与用户保护。
妥善保存主密码和备份独立口令，项目不提供密码重置服务。

首次主机指纹须独立核验，密钥变化阻断认证。Agent 转发默认关闭，开启后远端可以
请求 Agent 签名；撤销现有授权需关闭相关会话。RSA-SHA2 已改用 AWS-LC 后端并移除
原漏洞依赖的审计例外，该改动不等同于新密码学审计。

本检出目录尚未列出专用私密漏洞报告渠道，请通过已有可信私密渠道联系维护者后再
披露细节。公开 issue、日志和报告中不要发送真实凭据、Vault/备份或终端正文；复现
使用一次性材料。未签名候选包用于受控测试，并应核对构建记录中的 SHA-256。

应用内更新须通过内置公钥验签，且签名绑定的版本必须与清单一致；缺公钥时不可用，
更新清单不能替换信任公钥。生产签名私钥只放在受保护的 `release` GitHub Environment。
更新包签名不等同于 Authenticode。轮换与恢复见 [更新说明](docs/UPDATES.md)。
