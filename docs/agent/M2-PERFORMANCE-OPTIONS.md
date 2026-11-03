# M2 high-RTT concurrency implementation options

<!-- m2-front-current -->
> 2026-10-25 独立复核续作：当前 r5 为 510 输入；恢复记录写入失败后的旧清理权限和导出初始化回滚已修复，独立只读复核无遗留代码阻断。crypto-bigint/ctutils 已锁定 0.7.6/0.4.5；必需工程检查 18/18 已通过。UI 239/38 文件、原生 32、SFTP 布局 38 场景通过。r5 非前台 7/7 已完整复算、7/7 通过，旧 r3 的 7 组通过只保留其原源码身份。0.2.0 隔离工程包的便携、安装、升级、卸载和七份合成数据保留验证已通过，仍待候选签收。前台继续按维护者安排延期；实际桌面验收和候选签收仍开放，A 门保留，M2 仍 blocked。最新证据以 [本轮收尾](M2-FOLLOWUP.md) 为准，下方旧检查点为历史。
<!-- /m2-front-current -->

Date: 2026-10-20. **Approved by the maintainer:** allow at most three SFTP
subsystems on the same authenticated SSH connection. The bounded pool,
automated regression checks and configured OpenSSH workflow are complete;
quantitative performance acceptance is pending.
The maintainer also requires `docs/PLAN.md` to remain untouched.

## 2026-10-21 quantitative continuation

The new engine joins independent preparation metadata and the final
parent/target/verified-temporary snapshots; final checks still run inside
the original global publication gate. Full hashes and every original check
remain required. A high-RTT tree warmup now observes upload/download duration
ratios 1.460/1.397, but formal three-round medians remain a separate gate.
This is scheduling work within the approved design, not another policy
exception or a removal of verification.

Raw duplex TCP reproduces the automatic WSL localhost failure without SSH
or Capstan. The identical private-address exchange succeeds. The existing
explicit private relay therefore also supports zero injected delay, and
records the route and actual RTT; both product and fresh WinSCP references
use the same route. The original route remains recorded as failed.

The current pinned xterm hidden-selection guard and indexed byte loops
preserve parser/ACK semantics, complete checksums, probe counts and timing
endpoints. Formal M1 now passes local/mixed input p95 94.3/146.4 ms against
the original 104.6/156.9 limits. Formal low-RTT visible three-file 1 GiB
mixed load also passes warmup plus three measured rounds: upload/download
SSH p95 129.2/111.6 ms, complete hashes, focus, resources and cleanup. The
default-socket high-RTT visible run also passes all four rounds: measured
SSH p95 220.8/211.6 ms, full hashes, focus, resources and cleanup. Complete
transfer medians and resume are collecting.
See [current evidence](baselines/M2/raw/2026-10-21-quantitative/README.md)
for source/package identities and retained failed attempts. The historical
options and measurements below retain their original source scope.

The enlarged 4 MiB upstream relay socket attempts are retained separately:
one fails foreground focus in measured round 2, and a fresh attempt fails
warmup with bulk ACK timeout and slow SSH sibling probes despite complete
focus. A traced 64 MiB diagnostic does not reproduce that 1 GiB failure;
its cause is not established. The subsequent formal default-socket profile
is recorded in
[the registration](baselines/M2/raw/2026-10-21-quantitative/default-network-registration.json).
It completes warmup plus three measured rounds with upload/download SSH
p95 220.8/211.6 ms against 269.6988 ms; warmup is excluded from those values. Subsequent single-file, three-file and tree
comparisons all use the same OS-default socket configuration with fresh
matching references. Results from different socket settings are not pooled,
and this choice does not claim a product fix for the enlarged-socket failure.

The previous M2 contract required one active SFTP subsystem per authenticated
SSH connection (`M2.md` section 4.2). Three file workers share its request
dispatcher. The registered comparison runs three WinSCP background sessions
against those three product workers; the connection-count difference is
explicit in the measurement method. Both use the same OpenSSH fixture,
payload, algorithms, disk and injected 100 ms RTT.

## Evidence and limit

- The independent Windows OpenSSH observation in
  `baselines/M2/raw/2026-10-20-b-engineering/channel-window-r3/channel-window/channel-window.json`
  records an activated peer SFTP channel window of 2,097,152 bytes and maximum
  channel packet of 32,768 bytes. This is an observed server limit, not a
  performance acceptance result.
- The completed earlier 64 MiB three-worker smoke records upload data phases
  of 5,777.99 ms for WinSCP and 11,941.23 ms for Capstan; download is
  9,858.56 / 23,648.63 ms. All data hashes match. The product/reference ratios
  are approximately 0.484 / 0.417. These are setup samples, not the formal
  three-round 1 GiB acceptance matrix.
- The first completed 1 GiB-per-file reference warmup in
  `baselines/M2/raw/2026-10-20-b-engineering/files-1g-high-r4/workflow-matrix/samples.json`
  uploads 3,221,225,472 bytes in 62,527.22 ms and completes upload/readback
  verification in 194,443.69 ms. All three SHA-256 values match. Aggregate
  reference data throughput is approximately 51.5 MB/s; its 80% target would
  be approximately 41.2 MB/s if that rate holds in the measured median.
- With the observed 2 MiB peer window held constant, one channel has a
  window/RTT upper bound of approximately 21.0 MB/s at 100 ms, before framing,
  acknowledgement thresholds and storage costs. Three workers sharing that
  channel do not multiply the peer's window. This explains why adding local
  request concurrency alone cannot close that conditional gap. The actual
  fixed rounds remain the acceptance evidence; a warmup and this bound do
  not manufacture a measured median or prove every server has the same limit.
- Read-only qdisc counters during that warmup show continued byte progress
  and zero drops. WinSCP suppresses repetitive packet logs; an unchanged log
  tail alone is not proof of a stalled transfer. Earlier canceled attempts
  remain incomplete records and are not reclassified as successful samples.

The separate visible mixed-load smoke's first upload SSH probe was 630.7 ms;
the following nine held-ACK sibling probes were 122.2–174.8 ms. That sample
exceeds the 269.6988 ms frozen input p95 ceiling. Its cause and full measured
rounds still need investigation; adding SFTP channels is not presented as a
proven latency fix.

## Approved implementation

Allow a bounded pool of up to three SFTP subsystems on the same authenticated
SSH connection. This retains the shared SSH connection, pinned trust,
credentials and connection generations. The maintainer explicitly approved
changing the subsystem count from one to three in this conversation.
`PLAN.md` section 2.4 also mentions one SFTP channel; this approved exception
is recorded here and in `M2.md`/`M2-CONTRACTS.md`, while that file remains
unchanged under the maintainer's instruction. No other PLAN decision changes.

The implementation retains one primary browsing/control subsystem and
bounds all data work to at most three subsystem leases in total. File handles
stay on the subsystem that opened them. Reconnect, opaque capabilities and
retry policy continue to bind to the authenticated SSH connection/generation;
the browser does not choose connection or subsystem IDs. The global three
workers, 64 requests per file, 208 protocol requests, 16 MiB owned payload
admission and task/grant bounds remain enforced across the subsystem pool.

The pool must coalesce concurrent opens, reject stale generations, cancel and
join each owned subsystem, isolate ordinary subsystem failure, and release
all leases before existing idle connection reclamation. Required regressions
include one authentication with three active file jobs and sibling shells,
global resource admission, per-channel handle ownership, partial/retry
verification, cancel/error isolation, late responses and complete teardown.
The full performance and mixed-visible/resource matrices still follow with
their original thresholds. No performance pass is assumed from this proposal.

The obsolete single-subsystem 1 GiB matrix attempt `files-1g-high-r4` was
stopped through its owned `cancel.request` mechanism after this approval.
Its completed reference warmups and incomplete round are preserved; the
fixture reports `cleanedUp=true`, `networkCleaned=true` and no acceptance.
The approved change does not relax the 80% floor,
change the supported server, add extra authenticated TCP connections or edit
`docs/PLAN.md`.

## Implemented checkpoint

The pool, shared admission, cancellation/draining and session-limit fallback
are implemented with nine boundary regressions. Required checks, configured
OpenSSH and separated production/benchmark builds pass; see
[the implementation report](M2-B.md) and
[raw evidence](baselines/M2/raw/2026-10-20-sftp-pool/README.md).

The first pool high-RTT three-file smoke has full hashes and bounded recovery,
but its data ratios are only 0.681 upload and 0.470 download. These 64 MiB
samples are not formal 1 GiB medians and remain preserved. The subsequent
bounded duplex stall is traced to the client awaiting a socket write outside
its packet select loop. The corrected full-duplex path passes a bounded
in-process reproduction with sibling input/rekey and a complete diagnostic
visible smoke in both directions (SSH p95 232.7/203.3 ms). Diagnostic timing
is excluded from acceptance. See the
[transport continuation](baselines/M2/raw/2026-10-20-transport-finish/README.md).
The full fixed matrices and prior failed samples retain their original
thresholds; subsystem approval and the transport fix alone do not complete
performance acceptance.

Final uninstrumented localhost mixed load still fails after that correction;
the bounded regression does not close the real-load gate. A subsequent
explicit Windows-loopback/WSL-private relay comparison completes both
directions with the same final executable, both with and without diagnostics,
but both rounds fail the foreground requirement and provide no latency
acceptance. The route can now be selected explicitly with
`-NetworkRelay windows_private`; the original route remains the default.
Both clients and probes use the same recorded route and fresh references;
the frozen policy is unchanged. See the latest M2-B checkpoint for full
results and remaining gates. No particular OS fault is asserted by this
comparison.

The first 1 GiB explicit-route attempt was stopped after the upload warmup
missed the fixed target (data ratio 0.679; download 0.898). Both warmups have
matching full hashes and zero retained payload. Live TCP observations show
77,152 ms of send-buffer limitation on the Linux relay's single 4 MiB
socket during the 91,128 ms upload. A new explicit fixture option,
`-RelaySendBufferBytes 4194304`, permits a controlled per-socket comparison
with actual kernel values recorded for both clients. It leaves default
settings, sysctls, production budgets and policy thresholds unchanged.
This is a fixture-bottleneck hypothesis; it does not establish the cause
of the original localhost mixed-load failure. See the latest M2-B report
and `socket-baseline-analysis.json` in the transport evidence directory.

The subsequent high-RTT 128-file tree warmup exposes a separate metadata
overhead: product/reference total upload is 169,484.79/67,030.48 ms (product
warmup versus reference median, not a formal product comparison). The
already-buffered SFTP adapter emitted length and body in separate writes.
It now hands off the complete frame together, with partial-write/backpressure
regressions and all eighteen engineering checks passing. Global commit
serialization, hashes and approved limits are unchanged. Real current-wire
default-route headless load still stalls, so this is not a demonstrated fix
for that failure. Fresh same-route measurements establish any metadata
benefit; cancelled warmups remain in their original evidence directories.

The complete current-wire smoke still misses the tree upload target:
173,673.35/60,514.01 ms product/reference total (ratio 2.870); download is
57,686.85/43,786.81 ms (1.317). All hashes and cleanup pass, but these are
single warmups. The adapter's fewer stream writes have not demonstrated a
solution to the throughput gap. The tree-only helper bound is now 600
seconds because the complete workflow plus extra oracle exceeded 180;
formal bounds and frozen numerical criteria are unchanged. Full evidence
and the remaining engineering work are in
[the closeout record](baselines/M2/raw/2026-10-21-closeout/README.md).

The next bounded-preflight checkpoint overlaps independent target snapshots
in groups of three. Tree checks run ancestors before descendants, overlap
only peers and keep stable conflict ordering; errors join all admitted
native snapshots and start no next group. Three new wire-peer regressions
fail on the preceding implementation and pass after the change, with an
additional native-error/drain regression and all eighteen required checks
passing. No publication lock, complete hash, final parent/version check or
worker/request/handle/payload limit is relaxed.

The actual same-route/default-socket high-RTT tree warmup is now
131,651.13/67,523.09 ms product/reference upload (ratio 1.950), and
56,458.62/40,326.98 ms download (1.400). All 128 file hashes, thirteen
directories and cleanup pass. Preparation through the first queued
observation decreases from 66,006.80 to 25,595.08 ms across the two product
warmups. These are exploratory observations with fresh references, not
formal medians or attribution of every remaining round trip. Upload still
fails the original 1.5 gate. Further optimization must preserve all safety
checks and the global publication gate.

Current-source formal localhost tree does pass: sixteen samples covering
warmup plus three measured rounds in both directions, with total-duration
ratios 0.405/0.328. This does not resolve the high-RTT gap or original-route
mixed stall. The source manifest, complete raw samples and independent
recomputation are in [the preflight checkpoint](baselines/M2/raw/2026-10-21-preflight/README.md).
Foreground measurements remain deferred by the maintainer; no new policy
exception or PLAN edit is requested or implied.

The fresh current-source localhost 3 × 1 GiB formal comparison also passes
all sixteen samples, full hashes and cleanup. Upload/download throughput
ratios are 1.538/1.564 and total-duration ratios 0.583/0.632; absolute
throughput floors pass. Its interrupted predecessor remains an incomplete
record, with exact Linux fixture cleanup and an automatically blocked local
directory deletion documented separately. These local formal passes do not
close any high-RTT or mixed-load failure. See the
[current delivery index](baselines/M2/raw/2026-10-21-preflight/delivery.json).


## Formal quantitative continuation, 2026-10-22

The unchanged 495-input latency checkpoint passes the complete M1 and
both low/high-RTT default-socket visible matrices. Formal low-RTT tree
also passes (upload/download duration ratios 0.211/0.293). Formal
high-RTT tree upload remains above the frozen limit: 109,732.69 /
57,566.02 ms, ratio 1.906 > 1.5. Download passes at 1.314. Each tree
case retains sixteen complete samples, 2048 full hashes and cleanup.
These fresh formal medians supersede exploratory smoke ratios.

The interrupted first three-file high-RTT attempt is retained, and its
replacement collects fresh references and all rounds under the same
registered default socket configuration. No safety check, budget or
numerical threshold is relaxed. A metadata-window optimization exists
only as an unapplied, untested target-directory draft; it is not part
of the measured product or a claimed performance fix. Further
implementation requires regression tests and fresh affected evidence.


## Complete default-socket baseline, 2026-10-22

All scheduled formal cases now have complete records. Numerical
failures remain: tree-high-final-r1: upload data=0.601, duration=1.906 | files-high-final-r2: upload data=0.693, duration=1.159; download data=0.805, duration=1.257. M1, both visible matrices and
six actual 1 GiB resume cuts pass. See the [complete quantitative
result](baselines/M2/raw/2026-10-21-quantitative/QUANTITATIVE-RESULTS.md).

Saved counters show product upload send-buffer limitation on the
owned relay, while the download snapshot shows historical receive-
window limitation. They do not establish a complete root cause or
prove namespace send-memory/queue tuning is necessary. Observed
queue drops are zero at the saved times. Metadata and receive-buffer
control drafts are unapplied/untested, retain original defaults and
need regression tests plus fresh affected measurements. No threshold,
global publication gate, complete hash or budget is relaxed.
