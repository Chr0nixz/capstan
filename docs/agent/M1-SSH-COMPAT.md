# M1 SSH authentication and Agent compatibility

**Final closeout (2026-10-17): M1 is accepted.** Fresh real OpenSSH runs passed all 12 cases with each of Ed25519 and RSA server host keys; fixed visible mixed measurements and package/runtime delivery also pass. TCP_NODELAY is enabled for interactive input. Maintainer M0/M1 device acceptance is recorded separately. See [M1-B](M1-B.md) for current evidence; dated pending statements below are historical. No new Agent version or independent cryptographic audit is inferred.

This is the current code/test matrix for M1. “Automated” refers to generated test keys or an in-process SSH server; it does not mean tested against a real Windows service or external `sshd`.

## Remaining real-device acceptance (2026-10-16)

After automated closeout, the maintainer reported that all remaining real-device
tests passed and clarified that the original records were not saved.
[The acceptance record](baselines/M1/raw/2026-10-16-maintainer-full-manual/README.md)
extends the earlier priority result to the remaining M1 functional GUI/auth/Agent,
recovery and migration scope. These manual paths no longer await execution.
No new Agent/server version, individual case trace, independently checked package
hash, quantitative performance result or runtime-package manifest is inferred.
The missing performance evidence and M0 entry deliverables remain separate;
this is not a new automated run or an independent cryptographic review.
Dated pending-manual statements below describe earlier states.

## Closeout update (2026-10-16)

RSA private operations now use pinned AWS-LC through the documented
[ssh-key patch](../../vendor/ssh-key/CAPSTAN_PATCH.md). The vulnerable RustCrypto
`rsa` package and advisory exception have been removed; `cargo deny check all`
passes without an advisory ignore. This removes that dependency blocker; it is
not a claim that the upstream advisory was fixed or that this adapter received
an independent cryptographic audit.

Two new disposable OpenSSH 9.6p1 Ubuntu-3ubuntu13.20 fixtures passed **12 configured
scenarios each**, with [Ed25519 server keys](baselines/M1/raw/2026-10-16-closeout-sshd/report.json)
and [RSA server keys](baselines/M1/raw/2026-10-16-closeout-rsa-host-sshd/report.json).
Each run covers password, four Ed25519 encodings, five RSA encodings (OpenSSH
plain/encrypted, PKCS#1, PKCS#8 plain/encrypted), saved trust and changed-key
blocking. The fixture generates keys at runtime and removes its account, server
and credential files afterward. No scenario skipped; these runs did not start
Pageant or the Windows OpenSSH Agent. Their earlier evidence remains separate.

RSA imports validate all components before authentication. New signatures use
SHA-256/SHA-512, with altered-message and inconsistent-component rejection tests.
Private imports support 2048–8192-bit two-prime keys. SHA-1 signing and weak or
multi-prime private keys are rejected; legacy SHA-1 public verification remains
for host compatibility. See [M1-B](M1-B.md) for complete results and desktop
boundaries. Dated assertions below about missing RSA evidence/the old advisory
blocker describe the historical implementation.

## Maintainer priority acceptance (2026-10-16)

The maintainer reported that all tests in the current required P1/P2/P3 priority
scope passed. [The acceptance record](baselines/M1/raw/2026-10-16-maintainer-priority/README.md)
records the original feedback and its boundaries. P1 supplies native Vault and
password entry, interactive password SSH and restart/lock evidence; P2 supplies
live Windows OpenSSH Agent GUI selection/authentication, forwarding off/on/off,
remote signing, two shells and identity-loss/recovery evidence; P3 supplies GUI
changed-key blocking, explicit recovery and restart persistence. The previous
P1 step 4 failure is closed after the timeout/error-message fix.

These are maintainer-reported desktop results, separate from automated server
runs. Optional private-key GUI/service-restart/refusal paths, cleanup and current
service state were not individually reported. Broader compatibility and deferred
release checks retain their earlier scope. Dated statements below that these
particular priority paths lack maintainer acceptance describe the earlier runs.

## Host-trust confirmation timeout fix (2026-10-16)

The 15-second SSH handshake budget previously included the user's fingerprint
confirmation, even though the trust broker allows 120 seconds for that decision.
Waiting 20 seconds before accepting reproduced a failed connection. Host-key
verification now pauses the network budget and resumes its unused time after
verification; the trust broker's 120-second decision limit is unchanged. Failed
or dropped startup cancels its pending prompt so a late response cannot persist
trust. Stable backend error identifiers survive the terminal service; SSH
startup failures no longer fall back to the local-shell error message.

In-process regressions cover delayed acceptance, rejected/expired/dropped
attempts, zero authentication before trust, and the remaining network deadline.
`crates/capstan-core/tests/ssh_workflow.rs` also covers an isolated encrypted
Vault, trust persistence, wrong-password rejection, credential update and
restart. Its optional real-server test waits 20 seconds before accepting, opens
a shell, reopens its temporary Vault and reconnects without another prompt.
It reads only caller-provided disposable credentials and creates its own Vault;
it never opens the maintainer's Vault or edits `known_hosts`.

Run that extra workflow with `cargo test -p capstan-core --offline --test
ssh_workflow -- --nocapture`. Set `CAPSTAN_TEST_SSHD` and
`CAPSTAN_TEST_SSH_PASSWORD_FILE` as below, plus
`CAPSTAN_TEST_SSH_HOST_FINGERPRINT` to the independently verified SHA-256 host
fingerprint. With no target or password source it explicitly skips the external
connection; when both are supplied, a missing fingerprint is a configuration
error. This backend check does not replace native credential entry or desktop
P1 acceptance.

## Real compatibility update (2026-10-13)

[`Test-M1Sshd.ps1`](../../scripts/Test-M1Sshd.ps1) now supplies a disposable WSL OpenSSH fixture. [The latest real-server report](baselines/M1/raw/2026-10-13-201729-sshd-7b97b715de89/report.json) passed eight configured scenarios against OpenSSH 9.6p1 Ubuntu-3ubuntu13.19, including a real Windows Pageant 0.85 process. The table below retains the earlier coverage matrix; this dated section supersedes its claims that these particular paths have never contacted an external server.

| Path | Real automated evidence | Remaining boundary |
| --- | --- | --- |
| Password | Successful authentication, two shells on one transport, UTF-8, PTY resize and exit status. | Native Windows credential entry and desktop workflow still require acceptance. |
| OpenSSH Ed25519 | Runtime-generated unencrypted and encrypted keys both authenticate and complete the shell checks. | Other algorithms and versions are not covered by this result. |
| PKCS#8 Ed25519 | OpenSSL-generated unencrypted and AES-256-CBC encrypted keys both authenticate and complete the shell checks. | This does not establish all PEM/PKCS#8 algorithms or cipher combinations. |
| Windows Pageant 0.85 | Live identity selection, first-hop authentication and forwarded `ssh-add -T` signing succeed. Agent access fails in each of two separate non-forwarded connections surrounding the enabled connection. | GUI selection, empty-Agent, refusal and encrypted-key prompts remain separate acceptance paths. |
| Forwarded multiple shells | The real OpenSSH test initially rejected the second shell. Fixed by serializing and acknowledging one forwarding request per transport; the server provides the same forwarding socket to later shells. The concurrent in-process regression rejects duplicate requests and asserts one request and one authentication. | Host-checkbox persistence and live Windows OpenSSH Agent are not covered by Pageant results. |
| Changed host key | The disposable server is actually restarted with a second generated key. The client reports `host_key_changed`, emits old/new metadata and preserves the saved test trust. | Test trust lives in an ephemeral file, not the application Vault. Desktop trust management remains unaccepted. |
| Windows OpenSSH Agent | No successful service-backed result; the service was left Stopped / Disabled. | Do not interpret Pageant success as Windows OpenSSH Agent acceptance. |

The fixture binds only `127.0.0.1:22292`, creates a dedicated account, restricts the Windows credential directory ACL, and removes its server, account and credentials at the end of successful runs. It extracts Ubuntu packages when needed rather than installing or enabling the system SSH service. It does not modify production `authorized_keys`, `known_hosts` or sshd configuration. No test keys are checked in. One failed-run credential directory could not be deleted because automatic approval rejected deletion; its location and cleanup status are recorded in [`M1-B.md`](M1-B.md).

## Earlier matrix

| Path | Implemented behavior | Automated evidence | Maintainer evidence |
| --- | --- | --- | --- |
| Password | Password remains in Rust; server-key trust is checked before authentication; rejection returns a stable authentication error. | In-process russh server accepts/rejects passwords and counts authentication requests; unknown/replaced host-key tests check that trust precedes authentication. The optional `real_sshd` test accepts a password from a caller-managed test file. | `CAPSTAN_TEST_SSHD` is not configured in the current workspace, so the external test has not run. Windows/WebView2 end-to-end login remains untested. |
| Imported OpenSSH key | Rust parses and validates the key before storing it encrypted in the Vault. | Runtime-generated Ed25519 key authenticates to the in-process server. Generated OpenSSH keys cover encrypted/unencrypted parsing and wrong passphrase rejection. The optional `real_sshd` test can authenticate with a key file. | No broad OpenSSH version/algorithm matrix. |
| PKCS#8 PEM | Unencrypted and encrypted PKCS#8 PEM are parsed in Rust. | Runtime-generated Ed25519 PKCS#8 keys cover encrypted/unencrypted parsing and missing/wrong passphrase rejection. | No external `ssh-keygen`/OpenSSL matrix. |
| RSA | RSA auth and hash negotiation code is present through russh's `rsa` feature. | No Capstan RSA login compatibility result is claimed. | **Release blocked:** the locked `rsa 0.10.0-rc.18` dependency is reported by RUSTSEC-2023-0071 as vulnerable to a Marvin timing attack. Resolve the implementation before enabling RSA for release. |
| Windows OpenSSH Agent | Uses the `\\.\pipe\openssh-ssh-agent` named pipe. Saved Agent credentials contain provider and SHA-256 fingerprint, not private key bytes or identity comments. | Fingerprint selection and metadata redaction are tested. The optional `real_sshd` test can use a selected Windows OpenSSH Agent identity. | Real Windows OpenSSH Agent enumeration, signing, unavailable service and refusal paths remain untested because the optional test has not run here. |
| Pageant | Pageant identity provider is available on Windows; unavailable on non-Windows platforms. | Provider selection/error mapping is covered by Rust code paths, not by a live Pageant process. | Pageant installation/version, identity enumeration and signing remain untested. |
| Agent forwarding | Per-host checkbox defaults off and is enabled only for Agent credentials. A forwarding policy is part of the connection-pool key. Authorized sessions request forwarding; inbound Agent channels relay bidirectionally with bounded Tokio buffers. Without authorization, unsolicited channels are closed without connecting to a local Agent. | In-process russh server plus TCP Agent protocol fixture checks permitted request/response, disabled unsolicited channel closure, host-save credential enforcement and pool-key isolation. UI test checks per-host save/edit state. The optional `real_sshd` test can run remote `ssh-add -T` through a selected Agent. | Real Windows OpenSSH Agent + `sshd` signing test is the highest-priority manual check in [`M1-B.md`](M1-B.md); this workspace has not run the optional test. Policy changes apply to new connections; close existing forwarded shells to revoke access. |
| Changed host key and trust management | A changed key remains blocked before authentication. A separate read-only comparison notice shows the stored and presented algorithms/fingerprints; it has no accept action. The user can explicitly forget the stored trust only after out-of-band verification; the next connection returns to first-trust confirmation. | russh in-process server asserts zero authentication requests and emits old/new fingerprint metadata; core test verifies revision-checked trust removal and redacted list DTO; UI test verifies comparison with a dismiss-only action. | First trust, changed key, removal, external fingerprint verification and reconnect sequence have not been manually exercised in the Windows/WebView2 application. |
| Agent certificates | Certificate entries are omitted from the selectable identity list. | Identity filtering is exercised. | Not supported in M1. |

Private-key content and passphrases are never placed in React state or general IPC payloads. Agent forwarding is separate from authenticating the initial Capstan connection: the first hop still uses the selected Agent identity, while the host-level forwarding switch controls whether that remote shell can request further signatures.

## Optional real-sshd test

`crates/capstan-ssh/tests/real_sshd.rs` runs only when `CAPSTAN_TEST_SSHD` is set, in the form `user@host:port` (IPv6 must be bracketed). Otherwise it prints a `SKIP` line when run with `--nocapture`. The test uses a per-run in-memory trust repository, accepts only the configured target's first host-key challenge, and does not edit `known_hosts` or the server. It checks trust persistence before authentication, the configured authentication method, two shell channels on one transport, PTY resize, a UTF-8 marker, and exit status. An optional remote public-key path enables forwarding for the Agent-authenticated connection and verifies a signature with `ssh-add -T`.

Authentication inputs are supplied to the Rust test process using paths, not secret-valued environment variables:

| Variable | Meaning |
| --- | --- |
| `CAPSTAN_TEST_SSH_PASSWORD_FILE` | UTF-8 password file; one trailing line ending is removed. |
| `CAPSTAN_TEST_SSH_KEY_FILE` | OpenSSH or PEM private-key file, limited to 1 MiB. |
| `CAPSTAN_TEST_SSH_KEY_PASSPHRASE_FILE` | Optional passphrase file for the selected private key. |
| `CAPSTAN_TEST_SSH_AGENT` | `openssh` or `pageant`; the service must already be available. |
| `CAPSTAN_TEST_SSH_AGENT_FINGERPRINT` | Optional exact SHA-256 fingerprint; required if the selected Agent exposes multiple identities. |
| `CAPSTAN_TEST_SSH_AGENT_FORWARDING_PUBLIC_KEY` | Optional absolute POSIX public-key path readable by the remote test account. Enables forwarding and verifies `ssh-add -T` in a remote shell. |
| `CAPSTAN_TEST_SSH_TRUST_FILE` | Optional caller-managed ephemeral trust file. The normal run creates it after verifying in-memory trust was saved before authentication; it must not already exist. It contains only generated public host-key material and is not an application trust store. |
| `CAPSTAN_TEST_SSH_EXPECT_CHANGED` | Set to `1` only after rotating the dedicated fixture's key. Loads the ephemeral trust file and requires a changed-key error, comparison notice and unchanged prior trust. |

Run it with `cargo test -p capstan-ssh --offline --test real_sshd -- --nocapture`. When a target is set without an authentication source, the test fails as misconfigured instead of silently passing. Use a dedicated WSL/Linux account and ephemeral credentials; the remote commands disable shell history, run `printf` and `stty size`, and, when configured, run `ssh-add -T -- <absolute-key-path>`. This test does not replace the Windows/WebView2 workflow or separate live OpenSSH Agent and Pageant acceptance.

For a complete disposable real-server run from Windows PowerShell:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/Test-M1Sshd.ps1
# Optional: include a separate official, signed Pageant executable. An existing Pageant causes failure.
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/Test-M1Sshd.ps1 -PageantExecutable C:/test-tools/pageant.exe
```

The script requires x64 Ubuntu WSL root access, `apt-get`, `ssh-keygen` and OpenSSL; package downloads use the configured Ubuntu repositories. Current path validation expects a checkout named `capstan` mounted under `/mnt/<drive>/`. Its elapsed times include Cargo startup/build/lock waiting and are not SSH latency benchmarks. Output files contain test names, counts, versions and stable failure codes; remote shell output and credential contents are not written to the report.
