# M2 SFTP compatibility probes

<!-- m2-front-current -->
> 2026-10-25 独立复核续作：当前 r5 为 510 输入；恢复记录写入失败后的旧清理权限和导出初始化回滚已修复，独立只读复核无遗留代码阻断。crypto-bigint/ctutils 已锁定 0.7.6/0.4.5；必需工程检查 18/18 已通过。UI 239/38 文件、原生 32、SFTP 布局 38 场景通过。r5 非前台 7/7 已完整复算、7/7 通过，旧 r3 的 7 组通过只保留其原源码身份。0.2.0 隔离工程包的便携、安装、升级、卸载和七份合成数据保留验证已通过，仍待候选签收。前台继续按维护者安排延期；实际桌面验收和候选签收仍开放，A 门保留，M2 仍 blocked。最新证据以 [本轮收尾](M2-FOLLOWUP.md) 为准，下方旧检查点为历史。
<!-- /m2-front-current -->

> Historical 495-input quantitative baseline: all six private-route/default-socket
> file/tree matrices have complete sixteen-sample records. M1, both
> visible mixed matrices and all six actual 1 GiB TCP resume cuts pass.
> Numerical failures remain: tree-high-final-r1: upload data=0.601, duration=1.906 | files-high-final-r2: upload data=0.693, duration=1.159; download data=0.805, duration=1.257.
> Full hashes and fixture cleanup pass; this does not accept failed
> performance gates or enlarge platform/server support. Independent
> review findings, affected desktop acceptance and v0.2 candidate
> remain open. [Complete quantitative results](baselines/M2/raw/2026-10-21-quantitative/QUANTITATIVE-RESULTS.md)
> take precedence over dated collection notes below.

> Latest bounded-preflight checkpoint: independent metadata snapshots overlap
> in groups of at most three; ancestors are checked before descendants and
> admitted native snapshots are joined on peer error. Publication checks,
> complete hashes and approved limits are unchanged. Eighteen required checks
> pass (Rust 329/14/351, UI 195, native 30), with explicit unconfigured probes
> still counted separately from real execution. The current 0.1.0 review is
> `m2-review-873a32487d0e`. Actual SFTP/core passes; formal localhost tree
> retains sixteen complete samples and passes (total ratios 0.405/0.328).
> Formal localhost three-file 1 GiB also passes sixteen complete samples:
> throughput ratios 1.538/1.564, total ratios 0.583/0.632, absolute gates,
> all hashes and fixture cleanup pass. Interrupted reference warmup remains
> a separate incomplete record with documented owned cleanup.
> High-RTT tree warmup ratios are 1.950/1.400, with upload still failing 1.5;
> the latest default-route mixed failure remains unresolved. No foreground,
> platform/server expansion or complete performance acceptance is implied.
> See [current evidence](baselines/M2/raw/2026-10-21-preflight/README.md).

> Preceding complete-frame wire checkpoint: the production adapter hands each
> buffered SFTP prefix/body to the stream together; body bounds, opaque-handle
> conversion, partial writes, hashes and publication checks are retained.
> Eighteen fresh engineering checks pass (Rust 325/14/347, UI 195), and the
> then-current 0.1.0 review package is `m2-review-49d4325489a4`. That source's real
> performance remains to be measured; the dated loopback passes below refer
> to the preceding wire implementation. See [M2-B](M2-B.md).

> Current-wire follow-up: actual SFTP/core passes; default-route headless mixed
> progress still fails. The complete high-RTT 128-file smoke checks all hashes
> and thirteen directories, with zero final payload/jobs/pool and graceful
> fixture cleanup. Product/reference upload/download total duration ratios are
> 2.870/1.317; upload remains outside the 1.5 target. This is one warmup per
> direction/client, not a formal median or widened server/platform support.

> 2026-10-21 continuation: formal final-source loopback tree and three-file
> 1 GiB comparisons pass; the default localhost mixed stall is reproducible
> without WebView. Explicit-route headless progress passes full output/hashes
> but accepts no foreground latency. Fixture identity/descendant cleanup and
> single-file deadline/output handling now have adversarial and actual OpenSSH
> regressions. Interrupted/failed single-file references are not product
> comparisons. See [M2-B](M2-B.md) for current quantitative and cleanup limits;
> desktop/editor/shell, full performance and candidate gates remain open.

> B engineering continuation: live downloads now reauthorize the original path
> on a fresh scope, and persistent recovery binds original source attributes and
> authenticated-policy digest. A real tree probe exposed native rename path
> termination; filename-alignment and concurrent-tree regressions cover the
> correction. Three-file low/high-RTT, 128-file tree, six 64 MiB high-RTT TCP
> resume smoke cases, six complete 1 GiB loopback resume cases and the qualitative
> unread-shell/three-upload regression are indexed [here](baselines/M2/raw/2026-10-20-b-engineering/README.md).
> Visible successes and preserved timeout/foreground failures remain separate;
> they do not expand server/platform support or accept B performance.

> 2026-10-20 engineering completion: product native drop/export, reviewed directory/name/type decisions, safe download-partial reattachment, shell transactions and bounded recursive deletion now have code and regression coverage. Current checks and real-server evidence are indexed in [the completion record](baselines/M2/raw/2026-10-20-engineering-completion/README.md). Recursive deletion never follows symlinks, reports partial results and drains admitted removals before exit. Desktop/actual-editor and full quantitative acceptance remain pending. Older dated counts below retain their original scope.

> 2026-10-18 面板偏好/退出收敛增量后，另一次 [真实 OpenSSH 核心回归](baselines/M2/raw/2026-10-18-panel-preferences/core-final/report.json) 通过：protocol/core 退出 0、fixture 清理成功。它不代替偏好 GUI 手测，也不重写下方历史性能与兼容记录。

> 2026-10-18 · Development probe results, not a product support matrix or M2 acceptance.

Dependency: exact locally patched `russh-sftp 3.0.1`, Apache-2.0, used at runtime behind `capstan-ssh::sftp`; vendored `russh 0.61.2`. Sources: [published crate](https://crates.io/crates/russh-sftp/3.0.1), [upstream repository](https://github.com/AspectUnk/russh-sftp). The byte-stream interface avoids introducing a second russh version into the application.

## Automated observations

| Scenario | Observation | Product implication |
| --- | --- | --- |
| SFTP v3 / extension map / u64 attribute | Deterministic duplex probe preserves v3, advertised extension values and u64::MAX | Does not establish support for later protocol versions or extension semantics |
| Out-of-order, duplicate and unknown request replies | Outstanding requests receive their own results; other numeric IDs are ignored by this upstream release | Ignoring unknown replies is characterized behavior, not an assertion that malicious replies are always safe |
| Canceled request / late reply | A dropped request does not complete its successor | Does not resolve uncertain mutation outcomes |
| Stalled request / EOF | Timeout is bounded; EOF wakes the outstanding request | No automatic retry of destructive operations is added |
| Extension versions and advertised limits | Only exact version `1` enables atomic rename/fsync; packet/read/write limits clamp payloads; zero/unlimited and u64 maxima are handled; an invalid tiny packet is rejected | Unknown versions never authorize replacement; advertised max_open_handles is capped at 64 and excess OPEN fails locally without closing the channel; CLOSE permits reuse |
| Malicious packet length / truncated body | Stream fails without accepting a result | Adapter bounds allocation before reading a body |
| Non-UTF-8 names | Distinct byte names collapse to the same U+FFFD string upstream | Service skips unsupported entries and returns cumulative `omittedEntries`; the panel warns and keeps valid entries usable. Lossy names are never operation targets; raw-byte name support remains absent |
| Binary opaque handle | Raw upstream changes valid bytes | The framing adapter fixes HANDLE and supported outgoing handle fields; all-byte READ/WRITE/CLOSE test checks exact equality |
| Device permissions | Upstream `is_dir` accepts a block-device mode | Core uses the exact POSIX type mask. Recursive workflows must not call upstream's flag helpers |
| Untrusted STATUS text | Adapter removes arbitrary error message/language text while preserving the status code | Runtime must map status codes into stable domain errors |
| Two shells + deliberately stalled SFTP on one transport | In-process russh authenticates once, opens one SFTP, and the adjacent shell remains responsive before and after SFTP close | A protocol feasibility test; does not test ConnectionPool single-instance coordination or production terminal bindings |
| Production pool + service, two shell-bound views | Six browsing, fourteen transfer/attach, nine lifecycle and two bound cwd workflows use the actual pool, leases and SftpService | Includes native I/O/hash/conflict/failure/grant cases; rejected/invalid initialization, explicit recreation/stale capabilities, final lease/idle expiry with simulated time, queued/conflict/active disconnect, abrupt TCP EOF, dead alias/new generation, admitted commit/exit drain and cwd host/window/view/sequence/alias checks. Payload doubling is feature-gated; actual external cases run separately below |
| Production library File pipeline | A fully unacknowledged 64-WRITE window, reverse replies, a short-read gap, failed-ACK holes and stalled-file siblings/control are exercised over deterministic duplex peers | Only a contiguous successful prefix is progress. File drain/close retains admission through convergence; existing library dispatch/matching is reused. Request IDs fail on exhaustion instead of wrapping; complete mixed-load/fairness acceptance is still required |
| Backpressure / close / payload admission | Adapter cancellation joins both tasks while output is blocked; core enforces 16 MiB of owned payload admission before allocation and through native I/O | 512 KiB/1 MiB native upload/download peaks stay 4,259,840 bytes with the production window and return to zero. This is service-owned payload, not fixture storage or process RSS. Reordered/short-gap/failed-ACK peers and stalled sibling progress pass; full mixed-load acceptance remains B work |
| OSC 7 input contract | Shared hostile URI fixture validates configured/explicit hosts, bracketed IPv6, one decode, length/UTF-8/controls and scope | xterm split-binary parsing/ACK, bounded source coalescing, window/terminal/view/generation/sequence checks and explicit alias revocation pass. Per-tab Follow and interaction gating are implemented. Shell installation remains open; no local access or writes are granted |

Tests: `crates/capstan-ssh/tests/sftp_protocol.rs`, `crates/capstan-ssh/src/sftp/wire.rs`, core/TS metadata fixtures. The tests named `probe_*_lossy*` characterize upstream limitations deliberately; they are not passing product behavior without the adapter/validation layer.

The workflow fixture uses runtime-generated credentials/keys and a real in-process SSH transport with deterministic SFTP responses. The abrupt EOF test closes real proxy sockets while WRITE is stalled; no orderly SSH disconnect or automatic authentication follows. See [current Rust checks](baselines/M2/raw/2026-10-18-pipeline/rust-final/checks-rust.json): default executes 31 in-process workflows, all features add the payload file-size-doubling case (32). Default also reports one external case; all features report that case and the new product-performance case. These return early when unconfigured; that `ok` is not the separate OpenSSH execution below.

## Real OpenSSH

Runner: `powershell -NoProfile -File scripts/Test-M2SftpProbe.ps1 -EvidenceDirectory <evidence-dir> -CoreWorkflow`.

The runner uses the existing isolated M1 WSL fixture with an explicit `sftp` mode. Only that mode adds `Subsystem sftp internal-sftp`. It generates credentials and host keys at run time, pins the actual host public key, uses a random fixture account/root, enables real tests through `CAPSTAN_TEST_SSHD`, then removes the fixture and credential directory. Ordinary users' remote directories are not test cleanup targets.

The protocol probe checks authentication, subsystem reply, v3, realpath, directory handle/read/close, exclusive creation, 1 MiB write/read with complete byte equality including short reads, stat, remove and close. `-CoreWorkflow` additionally runs the actual production ConnectionPool/SftpService/TransferService against OpenSSH with two shell leases and Windows native local I/O. It uploads, explicitly replaces via the advertised extension, downloads and hashes 1,048,613 bytes, then deletes its remote test file. The lifecycle extension uses checked pool acquisition, rejects a closed view and explicitly recreates SFTP on the same authenticated transport before closing its leases. The core path exercises fsync and atomic replacement, without claiming power-loss durability or visible desktop/performance acceptance.

Current real execution after pipeline/cwd changes: [OpenSSH report](baselines/M2/raw/2026-10-18-cwd/core-final/report.json), [core log](baselines/M2/raw/2026-10-18-cwd/core-final/core-workflow.txt). OpenSSH 9.6p1 Ubuntu-3ubuntu13.20, protocol/core exit 0, cleanup succeeded. Complete SHA-256: `c2697d3e0319553e48e46cec5053d0788e63dc44ce8b3bf00b04c38cbd37016a`. This core case is low-latency correctness evidence, not interruption/resume or visible shell latency. New-generation transport replacement is covered by the in-process peer.

The current [folder-batch OpenSSH report](baselines/M2/raw/2026-10-19-folder-conflicts/openssh-r1/report.json) and [parsed core workflow](baselines/M2/raw/2026-10-19-folder-conflicts/openssh-r1/parsed-core-workflow.json) also exit 0 with fixture cleanup. The production workflow covers five files, upload batch replace, download batch replace and skip, selected-root preservation, empty-directory preservation, complete per-file SHA-256 verification and remote cleanup. `performanceAcceptance` and `desktopAcceptance` remain false; this is correctness evidence, not the full performance or GUI acceptance matrix.

The formal [1 GiB loopback](baselines/M2/raw/2026-10-18-a1/low-r1/library-performance/comparison.json) and [64 MiB controlled-high-RTT](baselines/M2/raw/2026-10-18-a1/high-rtt-r5/library-performance/comparison.json) upstream-library references use 64 requests, matching binary hashes, fresh data SSH connections on both clients, and one warmup plus three measured rounds per direction/client. All 32 full hashes match. Upload/download throughput ratios are 5.1893/1.6614 and 0.9309/1.3082 respectively; both pass unchanged 80% thresholds frozen before library samples. Actual high-RTT SSH echo p95 is 109.8494 ms; ICMP/qdisc records identify the isolated 50 ms-per-end injection. Main [numeric policy](baselines/M2/performance-policy.json) is now frozen; this is A1 feasibility, not production 1 GiB/high-RTT/three-worker/tree/mixed-terminal acceptance. The [A1 index](baselines/M2/raw/2026-10-18-a1/README.md) retains failed 16-request results, initial network setup failures, the earlier asymmetric connection method and corrected formal references.

Later [product loopback](baselines/M2/raw/2026-10-18-pipeline/product-low-r1/product-performance/comparison.json) and [product high RTT](baselines/M2/raw/2026-10-18-pipeline/product-high-r3/product-performance/comparison.json) runs use the actual production pool/services/native I/O and full verification/commit. For 1 GiB loopback, data upload/download ratios are 3.9631/1.7239 and total ratios 2.5042/1.6137. For 64 MiB high RTT they are 1.0285/1.1091 and 0.9934/1.0148. All 32 hashes, unchanged relative/absolute gates and fixture cleanup pass. Full 1 GiB high-RTT/three-worker/tree/mixed-terminal/resource/candidate acceptance remains open. Invalid suspend-crossing and launcher-failure runs are retained in [pipeline evidence](baselines/M2/raw/2026-10-18-pipeline/README.md).

Preserved first failures:

- [first attempt](baselines/M2/raw/2026-10-17-openssh/report.json): exit 101, fixture cleanup succeeded. The test compared public-key comments as identity; fixed to compare key material.
- [second attempt](baselines/M2/raw/2026-10-17-openssh-adapted/report.json): exit 101, fixture cleanup succeeded. The test assumed the first subsystem response was Success; fixed to allow SSH window adjustment before the reply.

Latest execution evidence is recorded in [M2-B](M2-B.md). Earlier attempts remain archived; do not interpret a generic unconfigured test `ok` as a real OpenSSH execution.

The transfer extension's own first two attempts are also retained under `2026-10-17-transfers/openssh/` and `openssh-r2/`: a temporary-lifetime compile error in test setup, then a test trust-constructor mismatch involving a public-key comment. The test now uses the production `HostPublicKey::from_openssh` normalization. Trust validation was not bypassed; successful r3/r4/r5 reports retain the earlier failures.

## Product integration status

The application has a shared-connection SFTP client, Rust terminal binding, restricted commands, a bilingual SSH panel, native-picker single-file upload/download, a global three-worker queue, conflict decisions, payload admission, the 64-request production file pipeline, transfer exit handling, bound per-terminal OSC 7 following, recursive folder pickers with empty-directory preservation and version-bound regular-file batch replace/skip, and Rust-owned remote editing/local startup recovery. The editor has automated native/core/metadata coverage; actual editor compatibility remains unaccepted. File bodies remain in Rust. Resource/commit boundaries are in [M2-CONTRACTS](M2-CONTRACTS.md); [M2-A](M2-A.md) now records accepted A1–A6, while [M2-B](M2-B.md) retains blocked B/M2 status.

After the delivered [manual checklist](M2-MANUAL.md), the maintainer replied “测试全部通过”. [The feedback record](baselines/M2/raw/2026-10-18-maintainer/README.md) accepts A-01–A-08, four-file complete hashes and the independent Explorer feasibility probe as maintainer-reported pass. A2–A6 preserve that scope; the added A1 engineering evidence closes the A gate. No new human result, raw trace, tested GUI build hash or platform/display matrix is fabricated. Automated reports retain their original scope and flags.

## Unverified / unsupported claims

Production SFTP drag-in/out for files/folders, directory/name/type conflict decisions, editor launch/save/recovery, Bash/Zsh installation/follow, non-OpenSSH servers, power-loss durability, full product WinSCP/high-RTT acceptance, 1 GiB interruption/resume, candidate installer/portable upgrade and quantitative visible-terminal performance remain unaccepted. Library high-RTT feasibility is separately measured above. The delivered native-picker/visible single-file and standalone Explorer checks are closed by the human report; that does not accept the newly implemented product dragging or extend the platform matrix. The request-ID boundary is tested, but the full request space is not traversed. Normal cancellation drains independently while retaining request/handle/payload admission. EOF, malformed protocol or an undrainable timeout can still quarantine SFTP and interrupt sibling file tasks; adjacent SSH shells remain independent.

## Remote editor implementation boundary, 2026-10-19

The supported input contract is bounded UTF-8/BOM-marked UTF-16 regular text with raw-byte preservation. Windows staging, protected current-user/SYSTEM ACL, rename-save snapshots, exclusive instance ownership, explicit local export/discard and core stability/conflict behavior now have automated evidence in [M2-B](M2-B.md). The global queue survives closing the source shell. Startup copies have no persisted remote authority and never replay an upload. VS Code/Notepad++ or another actual `.exe` launch, shared-process behavior and repeated saves still require B-edit acceptance with version/configuration recorded. Explicit new-generation retry preserves the authenticated policy and original baseline. The live process tracks remote temporaries and only removes matching attributes/full hashes, including after a verified rebind. That ledger now uses bounded current-user encrypted checkpoints with mirrored intents before OPEN, process-exit recovery and explicit policy-checked cleanup. Recovered documents have no upload authority. Unreadable metadata preserves the document; explicit retention allows exit without deleting or replaying anything. Native/core tests cover torn slots, tamper/context mismatch, hard links and lost-ACK recovery; actual editor acceptance remains separate. This does not change the blocked B gate or broaden Windows x64 support.
