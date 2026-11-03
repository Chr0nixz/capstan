# M2-B implementation report

> Updated: 2026-10-25. A1–A6 and the A gate retain their acceptance. B and M2 remain `blocked` on final performance and the v0.2 candidate; the maintainer reports affected desktop testing complete. Scoped independent code review is complete. No commit, push, tag or publication; PLAN and frozen numerical limits are unchanged.

<!-- m2-front-current -->
Current r5: engineering checks, independent review, seven nonforeground cases and engineering packages pass; desktop completion is maintainer-reported. Low RTT foreground passes. High RTT: INCOMPLETE; SSH ACK timeout in formal round 2 at 100% observed focus. M1 retains three numerical failures at 100% focus. M2 and candidate acceptance remain blocked. See [the current follow-up](M2-FOLLOWUP.md).
<!-- /m2-front-current -->

| Gate | Current evidence | Remaining requirement |
| --- | --- | --- |
| B6 file/tree/resume | Final r5: 7/7 complete/audited, 7/7 pass unchanged limits; prior r3 keeps its identity | Nonforeground execution complete; foreground and full B6 acceptance remain separate |
| B6 foreground M1/visible/resources | r5 M1: 100% focus, three numerical failures; low RTT complete/audited PASS; high RTT: INCOMPLETE; SSH ACK timeout in formal round 2 at 100% observed focus | Investigate M1 and the remaining high RTT result |
| M2-C §7 findings | Independent scoped review closes all five edges; maintainer reports affected desktop tests complete | No identified scoped code-review blocker |
| B1–B5/B7 desktop | Maintainer reports all six affected groups complete; no issues or per-case/platform matrix supplied | Completion recorded at the reported scope |
| B8 candidate | Version-only 0.2.0 engineering NSIS/portable, actual ordinary-user 0.1.0 upgrade/uninstall and seven-file preservation pass | Candidate promotion/sign-off after prerequisite B gates |

The dated sections below preserve their original selected source and package
identities. Their uses of “current”, “remaining” or test totals are historical;
the closeout above is the current implementation record.

## Completed 495-input quantitative baseline

Every scheduled private-route/default-socket formal case now has a complete record. The execution and saved-evidence audit are complete; numerical failures remain failures. Current failed cases: tree-high-final-r1: upload data=0.601, duration=1.906 | files-high-final-r2: upload data=0.693, duration=1.159; download data=0.805, duration=1.257.

See the [complete quantitative table](baselines/M2/raw/2026-10-21-quantitative/QUANTITATIVE-RESULTS.md), [raw recomputation](baselines/M2/raw/2026-10-21-quantitative/quantitative-evidence-audit.json) and [delivery index](baselines/M2/raw/2026-10-21-quantitative/delivery.json). M1 and both visible matrices pass their frozen gates; all six exact one-GiB TCP interruption/resume cases pass. No smoke, interrupted record or unapplied draft is substituted for formal evidence. The dated collection notes below retain their original scope and do not override this result.

## Dated engineering and measurement records

### Sixteen-peer metadata and controlled socket experiments, 2026-10-22–23

Production changes are confined to `capstan-core/transfer`: a private bounded
join admits at most sixteen independent metadata futures, waits for every
admitted result even when a peer fails, and returns results in input order.
Batch targets keep their own baseline versions. Tree planning still checks
ancestors before descendants and never looks up descendants of rejected
directories. Existing upload-tree parents are canonicalized first, then their
ancestor types are checked in joined groups; missing parents keep ordered
creation and final canonical validation. Final checks and publication remain
under the original global commit mutex. No dependency, worker, subsystem,
request/handle, hash or payload-budget increase is introduced.

Three existing preflight regressions now exercise 33 reordered targets in
16/16/1 groups, failure before a seventeenth lookup/grant, and draining sixteen
admitted native snapshots on peer error. Five new Rust regressions cover a
wider tree level and canonical-first parent validation, including reversed
ancestor replies, denied snapshots, an ancestor replaced after staging and
ordered creation of missing parents. Actual old-production failures are kept
separately from the initial test-only compilation error. The corrected focused
SFTP suite reports 96 passes; the full required checks pass 18/18 with Rust
337/14/359, native 30 and UI 197/34 files. Rust counts overlap and include
nine/twelve explicit unconfigured external skips; they are not real-server
execution counts. Seventeen tooling checks and thirteen relay tests on each
platform pass. See [verification](baselines/M2/raw/2026-10-22-quantitative-finish/verification.json).

The fixture adds an opt-in upstream receive-buffer control and an explicit
one-GiB headless progress diagnostic. Defaults, queues, host/namespace TCP
memory, individual progress deadlines and frozen numerical limits are
unchanged. The new-source default-socket formal high-RTT tree passes at
1.441413/1.268648 total-duration ratios. The explicit send/receive four-MiB
one-GiB headless diagnostic passes both directions, six full hashes and all
terminal, job, payload, session, pool and fixture cleanup checks. Its raw
terminal counters independently confirm two 100-MiB output streams, sixteen
input probes, one pause/resume per direction and zero final in-flight bytes.
This does not accept the still-collecting three-file throughput or new-binary
foreground gates. [Current numerical results](baselines/M2/raw/2026-10-22-quantitative-finish/QUANTITATIVE-RESULTS.md)
keep completed, cancelled and active cases separate.

Known risks and open work remain explicit. Socket creation-time settings do
not establish steady-state TCP limits or a common root cause for upload and
download; read-only observations are not whole-case zero-loss proof. The
one-GiB smoke attempt was cancelled before product samples because it used
the inherited small-file helper bound; full comparisons use the existing
formal path without pooling that attempt. Parent checks remain subject to
the already-documented server-side concurrent-mutation race, rather than an
atomic directory transaction. The three independent findings in M2-C section
7, affected desktop workflows and v0.2 candidate/upgrade verification remain
open. Production review is 0.1.0; no candidate, publication or PLAN edit occurs.

### Quantitative acceptance work, 2026-10-21

The completed 495-input source checkpoint is
[`latency-source-manifest.json`](baselines/M2/raw/2026-10-21-quantitative/latency-source-manifest.json),
with 495 selected inputs and four further changes after the hidden-renderer
checkpoint. Its manifest chain retains the earlier metadata implementation
and fixture changes. Independent preparation snapshots and the final
parent/target/verified-temporary snapshots are joined before proceeding;
the final snapshots still run inside the original global publication gate.
Three reversed-reply regressions fail on the preceding implementation and
pass on the change, including denied-parent and changed-temporary rejection.
No full hash, parent/version check, publication gate or budget is removed.

The newest real-server headless mixed run passes on the explicit Windows to
WSL private route with zero injected delay: both transfer directions,
100 MiB terminal output, eight same-connection sibling probes with held ACK,
three full 64 MiB hashes and zero final sessions/pool/payload/jobs. The
automatic localhost route still stalls. Raw full-duplex TCP reproduces that
failure without SSH or Capstan; the same 100 MiB exchange succeeds through
the private address. This identifies a route-dependent fixture limitation,
not a general OS fault or an original-route product pass. Both clients use
the same explicit route and fresh references in the subsequent matrices.

The pre-render-fix foreground runs retain valid focus, hashes, topology,
renderer/resource bounds and cleanup, but M1 input p95 fails. The explicit
route's local/mixed values are 112.0/160.9 ms against unchanged
104.6/156.9 ms limits. WebView2 is 157.0.4341.10; the accepted older
baseline used 156.0.4314.23. No threshold is recalculated. A short real
WebView2 CPU profile identifies approximately 2.7 seconds in hidden DOM
selection redraws repeatedly measuring zero-width characters. The profiler
wrapper times out after collecting its profile/report; this diagnostic is
not a formal accepted measurement.

A content-hashed pnpm patch to the still-pinned MIT xterm 5.4.0 defers hidden
selection drawing while retaining logical selection and requesting a full
refresh on resume. Published JavaScript, corresponding source and source
map are kept together. Four actual-browser regressions cover hidden output
and selection without DOM redraw, logical selection preservation, restored
selection, and clearing a selection while hidden. The first guard also
exposed a missing resume refresh; that failure is retained and fixed.
The final fourteen browser checks and frozen offline install pass.

The first formal run with the hidden-renderer fix improves observed mixed
input p95 to 146.2 ms and local bulk median to 13.282 MiB/s, but local
terminals inside that mixed run still reach 113.5 ms. That is the failing
local-input metric; the isolated local reference itself does not fail.
Its complete focus, integrity, resources and cleanup results are retained.
The next change replaces typed-array iterators with indexed byte loops in
the production frame checksum and benchmark checksum/marker ledger. Every
byte, marker, timing endpoint and integrity check remains. A separate Node
mechanism control produces identical digests for 1 GiB in 2788/808 ms;
this is not a WebView2 acceptance result. New offset-view/64 KiB corruption
and streaming ANSI/marker regressions pass; formal foreground results are
collected separately.

Fresh required checks pass 18/18; Rust default/runtime/all-feature reported
totals are 332/14/354 (overlapping; nine/twelve explicitly unconfigured
external probes), native 30, UI 197 across 34 files, and tooling 17.
The production review package is `target/m2-review-07191ca025dd`, version
0.1.0, and the opt-in measurement package is
`target/m2-benchmark-d228463a9ec4`. Package hashes match and production
rejects the measurement entry with exit 2. These are not v0.2 candidates.
See [`latency-verification.json`](baselines/M2/raw/2026-10-21-quantitative/latency-verification.json)
and the [quantitative evidence](baselines/M2/raw/2026-10-21-quantitative/README.md).
Final-source M1 now passes every frozen check: local input p95 inside the
mixed workload is 94.3 ms, overall mixed input p95 146.4 ms and held-ACK
sibling p95 30.9 ms. All focus, integrity, topology, resource and fixture
cleanup checks pass. Whole-process-tree peak working set is 957,423,616
bytes and handles 4929. This is the complete same-binary formal regression,
not the earlier failed run or the standalone Node mechanism control.

The low-RTT visible mixed matrix also passes one full warmup plus three
measured rounds in both directions, with three native 1 GiB transfers per
load, the pinned one-SSH connection, all full hashes and zero final counters.
Measured upload/download SSH p95 is 129.2/111.6 ms (limit 156.9); local p95
40.6/32.2 ms (104.6); held-ACK sibling p95 41.6/45.3 ms (1000). Each
direction contains 33 local, 36 SSH and 30 held-ACK measured samples;
warmup is excluded. Focus, resources and all fixture cleanup pass.
The [saved-evidence recomputation](baselines/M2/raw/2026-10-21-quantitative/visible-low-recomputation.json)
independently reproduces those nearest-rank p95 values from the three raw
measured rounds and checks all four rounds' complete file hashes.

The enlarged-socket high-RTT attempt `visible-latency-private-high-r3`
passes warmup and measured round 1, including complete hashes, focus and
resources. Warmup upload/download SSH p95 is 162.8/218.8 ms under the
unchanged 269.6988 ms limit. Round 2 fails the frozen foreground gate at
154/206 focused observations (74.8% < 80%); round 3 is absent. The fresh
`visible-latency-private-high-r4` instead fails during warmup with bulk
`ack_timeout`, SSH sibling probes at 1.6–2.6 seconds and 190/190 focused
observations. Both attempts clean up completely and remain failed.

A traced 64 MiB diagnostic passes both directions and full hashes, but
does not reproduce the 1 GiB failure or count as formal acceptance. Its
cause remains open. The subsequent complete default-socket run is
registered in
[`default-network-registration.json`](baselines/M2/raw/2026-10-21-quantitative/default-network-registration.json).
All subsequent formal high-RTT transfer comparisons use that same default
setting and fresh references; enlarged-socket results are not pooled with
them. Source, numerical limits, sample counts and all integrity checks are
unchanged. Full upload readback makes a foreground round approximately
eleven minutes; complete transfer medians remain separate gates.

The complete default-socket high-RTT run now **passes** warmup and all
three measured rounds. Combined upload/download SSH p95 is 220.8/211.6 ms
(limit 269.6988), local p95 28.8/30.4 ms (104.6), and held-ACK sibling
p95 216.5/211.3 ms (1000). Each direction has 33 local, 36 SSH and 30
held-ACK measured samples. All 24 complete 1 GiB file-hash records, focus,
one-connection topology, terminal integrity and zero final counters pass.
Peak process-tree working set across the four rounds is 793,202,688 bytes
and handles 4775; all fixture/relay/namespace cleanup and helper sleep
requests are released. Raw p95 and process-tree peaks are independently
recomputed in
[`visible-final-recomputation.json`](baselines/M2/raw/2026-10-21-quantitative/visible-final-recomputation.json).

The [post-foreground identity audit](baselines/M2/raw/2026-10-21-quantitative/post-foreground-source-audit.json)
confirms all 495 selected source inputs, four changed copies, eighteen
required check logs, seventeen tooling checks, twelve review package
files and three measurement identities remain unchanged. PLAN and the
numerical policy match their original hashes. This audit launches no
application or tests. Background transfer/resume cases use default
sockets and fresh matching references. The two formal tree cases have now
completed; their actual results take precedence over earlier smoke ratios.

The low-RTT tree matrix passes warmup plus three measured rounds in both
directions: product/reference median total upload is
1,580.53/7,485.50 ms (ratio 0.211), and download 1,231.97/4,209.85 ms
(0.293). High RTT retains a formal upload failure: median
109,732.69/57,566.02 ms, ratio 1.906 against the unchanged 1.5 maximum.
Download passes at 52,379.85/39,876.74 ms (1.314). Each case includes sixteen
complete samples, 2048 full hashes, thirteen checked directories per sample
and complete budget/job/pool/fixture cleanup. Saved arithmetic and coverage
are independently recomputed in [high-RTT tree](baselines/M2/raw/2026-10-21-quantitative/tree-high-formal-recomputation.json)
and [low-RTT tree](baselines/M2/raw/2026-10-21-quantitative/tree-low-formal-recomputation.json). This does not accept
the failed upload or apply large-file absolute floors to the small tree.

The first high-RTT three-file attempt stopped after all eight references and
two complete product warmups; the next product sample has no completed
record. The runner and children were absent at continuation, and WSL was
not running; the interruption cause is unknown. No measured product median,
child exit or graceful shutdown is invented. [Remediation](baselines/M2/raw/2026-10-21-quantitative/files-high-final-r1/interruption-remediation.json)
confirms the owned account, processes, listeners and namespace are absent.
Private residual directories are retained, including the Windows directory
whose recursive deletion automatic approval rejected with only
"blocked by policy". The new attempt uses fresh references and all fixed
rounds; partial warmups are not combined with it.

Current-source formal low-RTT single-file and three-file matrices
also pass all sixteen samples in both directions. Single-file
upload/download data ratios are 3.211/5.016, end-to-end throughput
ratios 4.151/3.863. Three-file data ratios are 1.686/1.893 and
product/reference total-duration ratios 0.528/0.558. All frozen
data/total/absolute targets, sixteen single-file hashes, forty-eight
three-file hashes and fixture cleanup pass. The single helper records
zero payload, completed task phases and an explicit pool drain; it
does not expose separate final job/pool counters. The three-file
helper records all three counters as zero. See
[single-file recomputation](baselines/M2/raw/2026-10-21-quantitative/single-low-formal-recomputation.json) and
[three-file recomputation](baselines/M2/raw/2026-10-21-quantitative/files-low-formal-recomputation.json).

The current-source resume refresh also passes all six exact 1 GiB
cases: uploads and downloads interrupted at 20%, 50% and 85% by
dropping owned TCP sockets. Every case explicitly uses a different
authenticated connection generation, validates the retained prefix,
appends only the remaining suffix, verifies the complete SHA-256,
removes the partial and converges payload/jobs/pool to zero. The
[saved byte-proof audit](baselines/M2/raw/2026-10-21-quantitative/resume-low-formal-recomputation.json)
recomputes every cut and suffix count; no new resume-speed threshold
is invented. Fixture/relay/namespace cleanup and awake release pass.

The formal high-RTT single-file matrix also passes sixteen complete
samples: upload/download data ratios 1.080/1.091 and total-duration
ratios 1.169/0.927. Conservative absolute rates are 19,823,639 and
10,587,766 bytes/s against frozen minima 10,963,186 and 5,495,610.
All complete hashes, zero final payload and fixture cleanup pass.
See [single-file high-RTT recomputation](baselines/M2/raw/2026-10-21-quantitative/single-high-formal-recomputation.json).
The replacement high-RTT three-file case is the sole unfinished
matrix; the complete tree upload failure remains a failed gate.

### Bounded metadata preflight, 2026-10-21

The preceding tree sample identified substantial time before the first
queued observation. This continuation changes only preflight scheduling in
`transfer/batch.rs` and `transfer/tree.rs`, with three integration-test
files. Independent batch target snapshots now overlap in groups of at most
three, retain input-to-version association and join every admitted snapshot
before returning a peer error. No grant/task is published on preflight
failure and the next group is not started. Native blocking I/O is awaited.

Tree scans check ancestors before issuing descendant lookups, overlap only
peers at the same depth and preserve original conflict IDs/order. Invalid
or conflicting directories still block the whole subtree. The global
publication gate, final parent/version/conflict checks, complete hashes,
one-SSH/three-SFTP policy and all worker/request/handle/payload limits are
unchanged. No dependency, vendor or UI source changes are included.

Three new peer regressions fail against the preceding implementation and
pass after the fix: replies are withheld until a bounded batch arrives,
then reversed; absent/existing targets retain their own baselines; an error
publishes no selection and stops the next group. An additional native-error
regression proves admitted snapshots finish before return, active work is
bounded and the fourth snapshot is not started. The complete SFTP suite
reports 88 passed cases, including its explicitly unconfigured external
probe skips; those skips are not actual OpenSSH execution.

Fresh full verification passes all eighteen required checks, Rust
default/runtime/all-feature reported totals 329/14/351 (overlapping, with
nine/twelve explicit external skips), native 30, UI 195 across 34 files and
all fifteen tooling checks. Evidence, exact before/after files and the
493-input/five-change source manifest are in
[`2026-10-21-preflight`](baselines/M2/raw/2026-10-21-preflight/README.md) and
[`verification.json`](baselines/M2/raw/2026-10-21-preflight/verification.json).
The new production review package is `target/m2-review-873a32487d0e`,
version 0.1.0; packaged hashes match and its measurement entry rejects with
exit 2. The separate measurement package is
`target/m2-benchmark-859d885653b4`. Neither is a v0.2 candidate; no
foreground measurement was launched. The identities are recorded in
`review-build/identity.json` and `measurement-build/identity.json`.

Actual OpenSSH SFTP protocol and the production core workflow pass on this
source. The same-route/default-socket high-RTT tree smoke completes all four
client/direction samples, all 128 file hashes, thirteen directories and
relay/network/sshd/awake cleanup. Product/reference upload totals are
131,651.13/67,523.09 ms (ratio 1.950); download totals are
56,458.62/40,326.98 ms (1.400). Upload's independent oracle adds 59,008.21 ms
outside the comparison. Peak owned payload remains 12,648,448/12,713,986
bytes and final payload/jobs/pool are zero.

Compared with the preceding complete-frame checkpoint's single warmup,
product upload falls from 173,673.35 to 131,651.13 ms. The observed interval
through initial preparation and the first queued observation falls from
66,006.80 to 25,595.08 ms; both comparisons remain exploratory, with fresh
reference timings and no statistical acceptance claim. This identifies a
real reduction in preflight overhead, but upload still misses the unchanged
1.5 duration ratio. The preceding 2.870 upload failure is retained, and the
localhost mixed stall remains unresolved. See the explicit
[`comparison`](baselines/M2/raw/2026-10-21-preflight/tree-high-comparison.json).

Current-source formal localhost tree passes: warmup plus three measured
rounds for both clients/directions retain sixteen samples, each with all
128 complete hashes and thirteen directory checks. Upload median
product/reference total is 2,947.61/7,276.65 ms (ratio 0.405); download is
1,273.22/3,883.34 ms (0.328). The unchanged tree duration gate passes;
large-file absolute throughput limits are not tree gates. The independently
recomputed results and current-source association are in
[`workflow-evidence-audit.json`](baselines/M2/raw/2026-10-21-preflight/workflow-evidence-audit.json).

The first current-source three-file localhost attempt lost its execution
session during reference warmup and produced no complete sample or final
report. Its actual protocol pass and partial logs are preserved, and its
exact owned Linux listener/account were removed. Automatic approval rejected
the combined cleanup command containing recursive local-directory deletion;
the private `target/m1-sshd-545355c48d85` directory is retained. The termination
cause is unknown; no timeout, product failure or pass is inferred. The
[`interruption record`](baselines/M2/raw/2026-10-21-preflight/files-loopback-final-r1/interruption.json)
includes exact cleanup identities and the retained credential directory's
exclusive current-user ACL.

The fresh
[`three-file localhost formal attempt`](baselines/M2/raw/2026-10-21-preflight/files-loopback-final-r2/workflow-matrix/comparison.json)
passes on this source: one warmup plus three measured rounds, sixteen
client/direction samples, each transferring three 1 GiB files with complete
SHA-256 equality. Upload/download data-throughput ratios are 1.538/1.564;
product/reference median totals are 57,219.83/98,089.02 ms and
31,698.33/50,152.91 ms (duration ratios 0.583/0.632). Aggregate product
throughput with the frozen phase allowance is 108,720,151/110,626,946
bytes/s. Data, total and absolute gates all pass without changing policy.
Final payload/jobs/pool are zero in every product sample; all fixture and
awake cleanup passes. The runner exits 0 and
[`final cleanup`](baselines/M2/raw/2026-10-21-preflight/runtime-cleanup.json)
independently confirms no owned Windows process, Linux fixture/account,
listener or current fixture directory remains. The interrupted attempt
is retained separately.

The final [delivery index](baselines/M2/raw/2026-10-21-preflight/delivery.json)
and [integrity audit](baselines/M2/raw/2026-10-21-preflight/delivery-audit.json)
bind the source, checks, packages, formal passes, failed warmup, interruption
and documentation. The selected source identity is not a Git diff or an
exhaustive reproducible-build claim.

Remaining current-source formal matrices, the throughput/stall work, deferred
foreground/M1/resources, affected desktop acceptance, independent review
and v0.2 candidate gates remain open. PLAN and the frozen policy are unchanged.

### Complete SFTP frame writes, 2026-10-21

The new high-RTT tree attempt retains eight complete WinSCP reference samples
and both product warmups, all 128 hashes and thirteen generated directories
checked. Reference median total time is 67,030.48/40,629.30 ms
upload/download; product warmups take 169,484.79/56,364.17 ms, exploratory
duration ratios 2.528/1.387. The additional upload oracle takes 58,788.19 ms
outside the compared total. The attempt was cancelled through its owned
request file to investigate the upload gap; no three-round product median
or formal pass exists. Both relays, namespace, sshd and awake-request cleanup
pass. All samples remain in
[`tree-high-r1`](baselines/M2/raw/2026-10-21-closeout/tree-high-r1/workflow-matrix/report.json).

Inspection found that `SftpWire::relay` already buffers/validates a complete
packet, then hands its four-byte prefix and body to the SSH stream as two
writes. Metadata packets can therefore become separate SSH channel messages
solely for framing. The adapter now prefixes that same buffer and uses one
`write_all`; it still accepts partial writes, uses backpressure, flushes each
frame and validates the same body-size, handle and data bounds. This adds
only the existing four-byte wire prefix to the buffered packet, without
another body copy. It changes no worker/request/handle/payload limit, global
publication gate, conflict/version check, full hash or authentication path.

A new regression fails on the original code with the observed two writes;
after the fix all six wire cases pass, including a one-byte-capacity stream
that exercises partial writes/backpressure and exact bytes across sixteen
frames. See `wire-metadata-before.txt` and `wire-tests-after.txt`. The helper
also now records a faulted output reader as a structured cleanup/deadline
failure; it does not let `GetResult` mask the original outcome. Fourteen
tooling checks pass, retaining all existing relay/identity/mixed negatives.

This is an implementation correction and a performance hypothesis. Real
current-source measurements must establish its effect; it does not
retroactively accept the cancelled tree or the localhost mixed failures.
Earlier loopback passes below identify the preceding implementation, not
this new wire checkpoint. Source identity and exact copies are frozen in
[`wire-source-manifest.json`](baselines/M2/raw/2026-10-21-closeout/wire-source-manifest.json)
and `wire-source/` (493 selected inputs, fifteen changes from the transport
checkpoint, of which this is the only production source file). Fresh full
checks pass in
[`wire-verification.json`](baselines/M2/raw/2026-10-21-closeout/wire-verification.json):
all eighteen checks, Rust default/runtime/all-feature reported totals
325/14/347 (overlapping, with nine/twelve explicit unconfigured external
skips), UI 195 tests across 34 files and native 30 tests. Separate
production-review/measurement builds follow this source checkpoint. The new
production review package is `target/m2-review-49d4325489a4`, version 0.1.0;
every packaged hash matches and its benchmark entry rejects with exit 2.
Its source/binary/manifest identities are in
[`wire-review-build/identity.json`](baselines/M2/raw/2026-10-21-closeout/wire-review-build/identity.json).
It is a review package, not the gated v0.2 candidate.

The new measurement package is `target/m2-benchmark-d102fd162c56`; its
capability check and source/executable identities are recorded separately in
`wire-measurement-build/identity.json`. No foreground window was started.
Actual OpenSSH SFTP protocol and production core workflows pass on the new
wire, but the original-path headless mixed diagnostic still fails bulk drain
in
[`wire-headless-loopback-r1`](baselines/M2/raw/2026-10-21-closeout/wire-headless-loopback-r1/report.json).
It receives all eight idle probes, releases ACK to zero in-flight/queued
bytes, then stops at 4,315,003 bulk bytes and interrupted file jobs around
5.6–5.9 MiB. Terminal/pool and Linux fixture cleanup pass, including four
owned processes terminated without escalation. Coalescing frames therefore
does not close the localhost stall; no particular OS cause is asserted.

The first current-wire tree smoke reaches its 180-second helper deadline
before producing a product sample; its failure record has
`processExited=true`, `outputDrained=true`, and no cleanup error. Both
references and complete relay/network/sshd cleanup are retained. The prior
tree warmup's 169.5-second total plus 58.8-second independent oracle exceeds
that generic bound. Tree smoke now has an explicit 600-second helper bound;
other smokes stay at 180 and all formal cases at 2400. Frozen performance,
operation and resource limits are unchanged. Updated tooling passes fifteen
checks, including the shipped deadline function for every workload.

The bounded repeat
[`wire-tree-high-smoke-r2`](baselines/M2/raw/2026-10-21-closeout/wire-tree-high-smoke-r2/report.json)
completes both directions, all 128 hashes, thirteen directories and cleanup.
Product/reference total upload is 173,673.35/60,514.01 ms and download
57,686.85/43,786.81 ms, single-sample duration ratios 2.870/1.317. Peak owned
payload is 12,648,448/12,713,986 bytes and final payload/jobs/pool are zero.
Upload's independent oracle adds 60,245.60 ms outside the comparison.
Correctness and cleanup pass, but upload still exceeds the unchanged 1.5
limit; the complete-frame change does not demonstrate a solution to this
gap. These are warmups, not formal medians. See the explicit exploratory
[`comparison`](baselines/M2/raw/2026-10-21-closeout/wire-tree-smoke-comparison.json).

Final identities are in
[`final-source-manifest.json`](baselines/M2/raw/2026-10-21-closeout/final-source-manifest.json)
and `final-source/` (493 selected inputs, sixteen changes from transport).
[`final-verification.json`](baselines/M2/raw/2026-10-21-closeout/final-verification.json)
links the eighteen fresh wire checks/builds and fifteen updated tooling
checks. Only the workflow harness and self-tests differ from the wire build
checkpoint; every compiled Rust/UI/vendor/dependency input still matches.
No background measurement remains running at delivery. Full-size current-wire
comparisons and throughput/localhost-stall optimization remain engineering
work, not maintainer manual tests.

An analysis of the existing tree sample is preserved in
[`wire-tree-phase-analysis.json`](baselines/M2/raw/2026-10-21-closeout/wire-tree-phase-analysis.json).
The upload task timer starts before tree resolution; its first observed
queued state is at 47,478.64 ms. That interval includes resolution and
preflight, not worker queue delay alone. Across the 128 files, sampled
preparing/running/verifying/committing spans have medians of about
442/671/1,211/220 ms. These per-file medians are not formal three-round
performance medians. In particular, verifying includes readback, gate wait
and final checks, so it cannot isolate commit-lock cost. Sequential preflight
snapshots and parent/final metadata checks are profiling targets; a safe,
measured optimization remains open. The global publication gate, full hashes,
conflict/version checks and budgets must remain enforced.

The evidence inventory and remaining gates are in
[`delivery.json`](baselines/M2/raw/2026-10-21-closeout/delivery.json).
[`delivery-audit.json`](baselines/M2/raw/2026-10-21-closeout/delivery-audit.json)
checks selected source inputs, all sixteen changed-file copies, the eighteen
required-check log hashes, review/measurement package identities and local
Markdown file targets. It also verifies that PLAN and the frozen policy
remain unchanged. Raw reports, failed attempts and source checkpoints are
indexed by [`evidence-manifest.json`](baselines/M2/raw/2026-10-21-closeout/evidence-manifest.json).
This audit is evidence integrity, not independent review or performance,
desktop or milestone acceptance.

### Final fixture ownership and measurement teardown, 2026-10-21

The interrupted serial queue is no longer running. Its high-RTT single-file
attempt retains four complete, hash-matching WinSCP references (warmup and
first measured round, both directions), then stops during the next upload;
no product samples or formal comparison exist. The later tree/three-file
entries in that queue never started. The owned namespace, Linux relay and
sshd were explicitly cleaned after the interruption; the original partial
logs stay unchanged. The Windows relay process was already absent, so its
graceful counter record cannot be recovered. See the separate
[`interruption remediation`](baselines/M2/raw/2026-10-20-transport-finish/single-high-final-r1/interruption-remediation.json).
Automatic approval rejected deletion of the verified retired
`target/m1-sshd-596fe4343a34` directory with `blocked by policy` and no more
specific reason. Its ACL-protected Windows files remain; the corresponding
test account and service are removed. This is retained cleanup work, not a
successful Windows workspace cleanup claim.

Implementation self-review tightened Linux process ownership further:
the generated account's numeric UID alone no longer grants permission to
signal a process, because containers can reuse that UID. The helper requires
the exact fixture sshd executable/title/config or benchmark script, or a
verified descendant. Proven PID/start identities survive reparenting but
cannot transfer to a reused PID; each signal still rechecks identity through
a retained pidfd. Thirteen Windows/WSL identity regressions pass. Twelve
real OpenSSH scenarios, including key rotation, also pass on the final
helper. A final real default-route mixed failure terminates four owned
processes without escalation and leaves zero terminal/pool registrations.
That diagnosis still fails bulk drain and does not close the transport gate.

The single-file measurement now shares `M2OwnedProcess.ps1` with workflow
measurements: retained handles, exact owned-tree teardown, cancellation,
bounded process/output completion and structured timeout/cleanup outcomes.
It handles a natural-exit race without replacing the original timeout with
a stale termination error, and rejects a completed sample observed beyond
its deadline. Reference loops also honor cancellation before starting the
next sample. Thirteen tooling checks pass, including actual single-file
wrapper success/timeout, owned child-tree timeout, natural-exit cleanup,
eleven relay/validator cases, thirteen cleanup identity cases and fifteen
synthetic mixed-verifier cases. The earlier loopback process-stop failure is
retained; the helper correction does not retroactively validate that attempt.

The pre-wire
[`closeout verification`](baselines/M2/raw/2026-10-21-closeout/verification.json)
links the unchanged eighteen required Rust/UI/native checks by exact source
and log hashes, plus the freshly executed script and real-server checks.
Only five fixture/measurement scripts changed after those required checks;
application/library/UI, dependencies, tests and required-check runner inputs
remained identical at that helper-only checkpoint. Its
[`source manifest`](baselines/M2/raw/2026-10-21-closeout/source-manifest.json)
freezes 493 selected inputs, fourteen changes from the transport checkpoint,
with exact changed copies in `source/`. PLAN and policy hashes are unchanged.

### Headless mixed diagnosis and failed-run cleanup, 2026-10-21

The maintainer deferred foreground measurements. An explicit
`Test-M2SftpProbe.ps1 -HeadlessMixed -ProbeMiB 64` now exercises the production
Rust SSH/terminal/transfer services without a window: one authenticated
connection, bulk and idle shells, held ACK until backpressure, eight sibling
input probes while all three file jobs are Running, bulk ACK release and
drain, three full file hashes, then terminal/connection cleanup. It is a
bounded progress diagnostic; it does not measure input latency, rendering,
whole-desktop resources or accept any visible/M1 gate. Its flags reject
incompatible measurement families and unbounded inputs.

The original localhost attempts
[`headless-loopback-r1`](baselines/M2/raw/2026-10-20-transport-finish/headless-loopback-r1/report.json)
and [`r2`](baselines/M2/raw/2026-10-20-transport-finish/headless-loopback-r2/report.json)
both fail bulk drain. R2's numeric diagnostics show successful eight sibling
responses, released ACK (zero in-flight/queued terminal bytes), then bulk
stopping at 5,811,218 bytes with all three file jobs stopping around 3.5–3.8
MiB before timing out. No terminal content is logged. The test preserves the
primary failure separately from cleanup errors. The real localhost stall
therefore remains reproducible without WebView; its cause is unresolved.

The same test/source on the explicitly recorded Windows/private relay and
100 ms injected RTT passes both directions in
[`headless-private-high-r1`](baselines/M2/raw/2026-10-20-transport-finish/headless-private-high-r1/headless-mixed/report.json).
Each direction retains eight responses, seventeen confirmed all-Running
checks, complete 100 MiB bulk output, three matching SHA-256 hashes and zero
final admitted payload/jobs/terminal registrations/pool entries. Peak owned
payload is 12,779,520 bytes. Both relays, namespace and sshd cleanup pass;
socket settings are defaults. This narrows the diagnosis without proving an
OS fault or accepting the default route or foreground latency.

The first failed headless run exposed sshd fixture cleanup that stopped only
the listener and left authenticated children/writers using its generated
account. Its failure report stays unchanged; exact owned PID/executable/
account remediation is separately preserved. `m1-sshd-cleanup.py` now checks
the exact generated token, stored account, account home/UID and server path,
then terminates only that disposable account's processes and its verified
root listener/privileged monitors. Retained Linux pidfds and a repeated
identity/start-time check prevent PID reuse from redirecting a signal.
Shutdown has a deadline and an owned-process-only escalation. Missing PID
files no longer prevent cleanup; `sshd-stop.txt` is retained by subsequent
probe runs. A real host-key rotation then exposed OpenSSH's rewritten
listener title, which the initial helper missed despite reporting zero
matching processes. The new tenth regression reproduces that failure;
listener matching now tokenizes only that verified OpenSSH listener title
and still checks the exact config path, executable and root UID. The two
affected listeners were independently identified and stopped through retained
pidfds; original reports remain unchanged, with remediation recorded in
`sshd-listener-remediation.txt`. Ten adversarial identity regressions pass
on Windows and WSL. Workflow tooling passes eleven checks, including those
cleanup tests, eleven relay/cleanup-validator tests and the fifteen
mixed-verifier synthetic cases. Headless R2's terminal/pool cleanup is
valid; its initial `cleanedUp` flag alone did not prove listener cleanup.

The final source then repeats the original-path failure in
[`headless-loopback-final-r3`](baselines/M2/raw/2026-10-20-transport-finish/headless-loopback-final-r3/report.json).
Its primary failure remains bulk drain; the fixed cleanup automatically
terminates four owned processes without SIGKILL and retains
[`sshd-stop.txt`](baselines/M2/raw/2026-10-20-transport-finish/headless-loopback-final-r3/sshd-stop.txt).
The two host-key families also pass twelve actual OpenSSH scenarios each,
including host-key rotation and changed-key rejection, with one listener
terminated at each rotate/stop. See `sshd-cleanup-auth-ed25519-r2` and
`sshd-cleanup-auth-rsa-r1`. The failed earlier rotation and its remediation
remain preserved. A Windows-only path-separator error in the added test is
corrected; the Linux production cleanup behavior is unchanged by that test
correction.

The Linux relay's final-counter validator also now uses explicit rejection
instead of Python assertions. Its regression executes the shipped snippet
under `python -O`: the original code falsely accepts five invalid reports;
the corrected eleven-case suite passes on both platforms, including missing,
duplicate, malformed, negative and nonzero cleanup/admission records.
Sources and numeric diagnostics are preserved in
[`headless-r2-source-manifest.json`](baselines/M2/raw/2026-10-20-transport-finish/headless-r2-source-manifest.json).

The final continuation freezes 493 selected source/config inputs in
[`engineering-source-manifest.json`](baselines/M2/raw/2026-10-20-transport-finish/engineering-source-manifest.json),
with twelve script/CI/integration-test changes from the transport checkpoint
and exact copies in `engineering-source`. Application, library, UI and
production/measurement executable identities remain at the prior checkpoint;
no v0.2 version or candidate is created.
[`engineering-verification.json`](baselines/M2/raw/2026-10-20-transport-finish/engineering-verification.json)
records all eighteen required checks passing. The original full sweep retains
its two failures: a feature-gated test doc-markdown lint, corrected and
rechecked, and an isolated browser connection error after fourteen SFTP
scenes, followed by a complete successful standalone rerun. Neither failed
log is overwritten. Reported Rust default/runtime/all-feature totals are
323/14/345 (overlapping); default/all-feature logs contain nine/twelve explicit
unconfigured external-probe skips, which do not count as actual SSH runs.
Native tests total 30; UI has 195 tests across 34 files. The unchanged
vendored 142-case evidence remains separately retained. Final tooling is
`headless-tools-final-r2`: eleven checks, eleven relay/validator tests, ten
cleanup identity tests and fifteen mixed-verifier cases.

### Fixed matrices without foreground requirements, 2026-10-21

The final production implementation passes the loopback tree matrix:
one warmup and three measured rounds for each direction/client, 128 files
including empty/hidden/Unicode names, full hashes and all generated
directories retained. Median product/WinSCP total duration is
2,689.95/7,162.18 ms upload and 1,094.86/3,853.17 ms download, duration
ratios 0.376/0.284 against the unchanged maximum of 1.5. All sixteen
samples and cleanup/awake-request release are retained in
[tree-loopback-final-r1](baselines/M2/raw/2026-10-20-transport-finish/tree-loopback-final-r1/workflow-matrix/comparison.json).
This closes that quantitative case only. It does not accept the high-RTT,
visible/M1/resource, desktop or candidate gates.

The 3 × 1 GiB loopback file matrix also passes all sixteen samples and the
unchanged data/total/absolute limits. Upload/download median data throughput
ratios to WinSCP are 1.601/0.942. Product median data times are
17,304.55/31,162.01 ms against 27,709.18/29,365.07 ms reference times;
product total times are 44,931.90/33,360.59 ms against
58,225.51/32,284.78 ms. Full SHA-256 matches all three files in every
sample; admitted payload peaks at 12,779,520 bytes and returns to zero.
Jobs, disconnected pool and fixture cleanup converge. See
[files-loopback-final-r1](baselines/M2/raw/2026-10-20-transport-finish/files-loopback-final-r1/workflow-matrix/comparison.json).
This uses the original localhost path without the optional relay/socket
setting. It does not resolve the separate mixed-terminal failure.

The separate 1 GiB single-file production loopback comparison retains all
eight complete WinSCP warmup/reference samples and matching hashes, then its
first product upload exceeded the probe's 900-second deadline. The test's
internal transfer deadline also elapsed; no product median or comparison was
produced. Its attempt helper then reported `Access is denied` while stopping
the owned probe process. Fixture/network cleanup, report writing and helper
awake-request release still completed, but the overall matrix correctly
remains failed. See
[`single-loopback-final-r1`](baselines/M2/raw/2026-10-20-transport-finish/single-loopback-final-r1/report.json).
Do not treat the references as product samples. The process-stop race/rights
path needs a separate fixture-helper correction before repeating any affected
matrix. The interrupted queue and subsequent helper correction are recorded
in the final continuation above; no later queued result is assumed.

### Bounded relay socket configuration and CI, 2026-10-21

The first explicit-route 1 GiB attempt retains eight reference samples and
both product warmups. Upload/download warmup data times are
91,127.65/135,826.46 ms against fresh reference medians of
61,849.95/122,004.92 ms, exploratory ratios 0.679/0.898. Product hashes match,
payload peaks at 12,779,520 bytes and returns to zero. These are warmups,
not three measured product medians. The owned cancellation request stopped
the next upload and both relay/namespace/sshd cleanups pass; the attempt
remains failed and its raw samples are preserved.

Live TCP evidence shows a 4 MiB Linux relay send buffer and 77,152 ms of
send-buffer limitation during the 91,128 ms upload. This supports a fixture
bottleneck hypothesis but does not establish the cause of every throughput
gap or the original localhost stall. An explicit
`-RelaySendBufferBytes 4194304` option now requests that size on each owned
Linux relay upstream socket toward sshd; Linux normally accounts twice the
request. Actual minimum/maximum sizes and configuration failures are
recorded, and both clients use the same setting. Accepted sockets toward
Windows keep their OS defaults. Defaults and host/namespace sysctls remain
unchanged. Product worker/request/handle/payload limits are unchanged.

Two earlier 64 MiB experiments applied the request to both relay sockets.
Upload data ratios improve to 0.823/0.990, while download ratios regress to
0.496/0.644. All hashes and graceful cleanups pass, but neither sample is
performance acceptance. The broader socket setting is no longer the
implementation; the upstream-only comparison is a separate experiment.
The original sources and results remain in `delivery-source` and
`socket-both-sides-comparison.json`.

The separate upstream-only smoke still misses the download target: data
ratios are 0.912 upload and 0.575 download, with all hashes and graceful
cleanup passing. The actual upstream send buffer is 8,388,608 bytes; the
accepted socket begins at its unmodified 87,040-byte OS value. See
`socket-upstream-comparison.json`. The option remains explicit and useful
for diagnosis; it is not a demonstrated throughput fix and is not enabled
by default. Further high-RTT work must preserve these failures and use
fresh references with the same documented settings.

The relay suite now has ten cases on each of Windows and WSL, including
actual socket-value comparison and rejected unowned/out-of-range settings.
Workflow tooling still passes ten checks, including the fifteen synthetic
mixed-verifier cases. Linux cleanup now also requires the relay's final
zero-client/zero-writer/zero-configuration-failure record.

Windows CI now explicitly installs pinned Python 3.12.9 and runs the
workflow/relay self-tests, all-feature Clippy, native Windows checks and
both terminal/SFTP layout suites. YAML parsing and seven structural
command assertions pass locally; this does not claim a GitHub CI run.
All eighteen required engineering checks also pass again in
[delivery verification](baselines/M2/raw/2026-10-20-transport-finish/delivery-verification.json):
Rust default/runtime/all-feature totals are 323/14/344 (overlapping), native
tests total 30, and UI tests total 195 across 34 files. Unconfigured external
probes retain their explicit skips. The unchanged vendored 142-case suite
retains its separately hashed execution evidence.
The [upstream socket source manifest](baselines/M2/raw/2026-10-20-transport-finish/socket-upstream-source-manifest.json)
records 489 selected inputs, with six scripts and one CI file changed from
the transport checkpoint. Application/library/UI sources and the existing
production/measurement binary identities still match that checkpoint.

### Explicit relay fixture and controlled comparisons, 2026-10-21

The final application and library sources are unchanged from the transport
checkpoint below. The remaining localhost mixed-load stall was compared
against an explicit Windows loopback listener forwarding through an assigned
WSL private address to the same isolated netem/sshd. Product, WinSCP, seeding
and probes all use that route. Two 64 MiB executions, with diagnostics on
and off, complete both directions, all six hashes, 47 all-three-Running
checks per direction, terminal output and disconnect cleanup. Product
payload peaks at 12,779,520 bytes and returns to zero. Both executions still
fail the unchanged foreground requirement: 19/205 and 54/203 samples are
focused. Neither is latency acceptance. The maintainer asked to finish
other engineering first and defer foreground measurement.

The repeated routing comparison supports isolating the test transport; it
does not prove a particular Windows/WSL defect or close every product
mixed-load failure. Original-path failures remain preserved. The earlier
private-address benchmark edits and separate socket-writer experiment were
both reverted; no temporary address or writer task is in production code.
The latter experiment still failed on the original route. The retained
vendored suite has 142 cases, not the experimental 143.

`Test-M2SftpProbe.ps1 -NetworkScenario controlled_high_rtt -NetworkRelay
windows_private` now explicitly selects this fixture route. Its default
remains `wsl_localhost`. No product endpoint, SSH connection count, frozen
threshold or PLAN decision changes. The route and runtime-selected WSL
address, helper identity, Python version and cleanup are recorded. Each
relay admits at most 16 clients and forwards bounded 64 KiB chunks with
backpressure and half-close support; fixture and OS buffers are separate
from product payload admission. Shutdown cancels/joins owned clients and
checks zero remaining writers. A forced stop fails cleanup. No process is
stopped by name, and no traffic contents are logged.

Eight transport/configuration/lifecycle regressions pass on both Windows
Python 3.12.9 and WSL Python 3.12.3. They cover 8 MiB each way with hashes,
half-close replies, slow-reader sibling progress, 16-client admission,
refused peers, ownership validation and shutdown. Initial failures exposed
waiting for the listener before closing accepted transports and leaving
blocked Windows output draining during shutdown; both are corrected and
the failures retained. The final workflow tooling reports ten passing
checks, including the existing fifteen adversarial mixed-verifier cases.
Actual high-RTT OpenSSH protocol/core workflows and graceful relay/namespace
cleanup pass. An earlier startup failure selected multiple Python paths;
the helper now retains the first resolved application.

Implementation self-review checked literal process arguments, private-address
validation, token-bound paths, exact owned-process shutdown, independent
duplex pumps, bounded admission and cancellation joins. No production SSH
or credential authority is added by the fixture. This is the implementer's
self-review; independent review of the changed implementation is still a
separate gate.

The separate three-file smoke completes hashes and resource release. Its
WinSCP/product data times are 5,682.93/7,719.57 ms upload and
9,774.48/9,651.87 ms download (ratios 0.736/1.013); total times are
18,472.21/20,069.04 and 12,920.68/14,522.85 ms. These are single 64 MiB setup
samples. The first 1 GiB attempt and the subsequent explicit socket-buffer
comparison are recorded above, with fresh same-route references and
unchanged limits.

The [relay source manifest](baselines/M2/raw/2026-10-20-transport-finish/relay-source-manifest.json)
records 488 identities and six script changes. All application/library/UI
files still match the already verified production/measurement builds below;
there is no application rebuild for this fixture-only continuation. The
[evidence index](baselines/M2/raw/2026-10-20-transport-finish/README.md)
separates diagnostic attempts, source copies, tooling checks and formal
matrices. Full file/tree/single-task/resource/M1 performance, affected
desktop/editor/shell acceptance, independent review and candidate delivery
remain gates; no M2 acceptance is inferred from the relay comparisons.

### SSH transport progress and input shutdown, 2026-10-21

The post-ACK visible stall now has a transport diagnosis and a deterministic
regression. Live snapshots show unread application TCP bytes with no netem
drops; a numeric diagnostic identifies the SSH loop waiting for socket flush
outside its packet select. The client now polls pending writes alongside
packet reads, consumed-byte window credit and liveness timers. Interrupted
flushes retain their written cursor, preventing duplicate ciphertext. Bounded
application batches and control backlog include already admitted data deferred
by remote windows; window-adjust replay does not charge that payload again.
The original loop fails the new 1 KiB duplex fixture at 10 seconds. The fixed
loop exchanges 8 MiB each way and preserves sibling echoes and repeated rekey.

Async terminal shutdown also cancels a blocked input write or flush and ends
queued input without replay, with both blocked stages covered by regression.
Benchmark deadlines now include a hung input IPC and asynchronous readiness
predicate; four added UI tests cover timeout, cancellation and late completion.
Diagnostic-only state/count sampling never logs terminal contents or secrets.
The transport patch and its buffer/testing boundaries are documented in the
[vendored patch record](../../vendor/russh/CAPSTAN_PATCH.md).

The corrected diagnostic high-RTT smoke completes both directions, all 47
Running checks per direction and all six hashes on one SSH connection. Jobs,
payload and disconnected pool entries return to zero. Its SSH input p95 is
232.7/203.3 ms for upload/download, within the unchanged smoke ceilings;
payload peaks at 12,779,520 bytes. These diagnostic 64 MiB setup timings are
excluded from formal performance acceptance. Intermediate failures and the
original 630.7 ms historical smoke remain preserved. Full-size measurements
must still use the frozen sizes, rounds and thresholds.

The final uninstrumented smoke still fails: all 23 input probes return, but
the first two SSH samples are 1,531.2/1,181.4 ms and file work subsequently
fails. A repeat diagnostic on that executable also stops file and terminal
progress. Further numeric stages show the revised loop waiting in socket
select rather than inside a packet handler; a live WSL-side TCP snapshot
again records unread bytes. The remaining real-load failure is unresolved.
The earlier successful diagnostic cannot supersede these final failures.

The continuation's [evidence index](baselines/M2/raw/2026-10-20-transport-finish/README.md)
records source identities, the failing-before regression, corrected tests,
diagnostic limitations and cleanups. All 18 required checks pass across the
linked final runs: Rust default/runtime/all-feature totals are 323/14/344
(overlapping), UI 195 cases in 34 files, native 30 and vendored SSH 142.
Both host-key families pass 12 real OpenSSH authentication/trust scenarios,
and the SFTP/core workflow and seven tooling checks pass. Unconfigured
external returns retain 9/11 default/all-feature `SKIP:` records, not actual
external execution. The production 0.1.0 review is
`target/m2-review-db57a2fd86b2`; its
[identity](baselines/M2/raw/2026-10-20-transport-finish/review-final/identity.json)
records desktop SHA-256
`1E81D6745BF31D5F4F2FE7F627C9162BE691149FCF9FA2DE0744553C9D59C6C4`
and production measurement rejection (exit 2). It is not a v0.2 candidate.

The bounded duplex and blocked-input shutdown regressions are fixed, while
the actual mixed-load blocker and full quantitative work remain engineering
responsibility. This does not mark B/M2 accepted or transfer that work to
maintainer hand testing. The prior pool checkpoint below retains its
historical scope.

### Approved three-subsystem pool, 2026-10-20

The maintainer explicitly approved at most three SFTP subsystems on the same
authenticated SSH connection. The shared browsing/control primary counts
toward that total; three global file workers acquire exclusive data
assignments only after worker admission. Additional clients open lazily and
are retained by the primary for reuse. Per-slot weak caches avoid a strong
connection cycle. The approved count exception, including PLAN section 2.4,
is recorded in [M2](M2.md), [contracts](M2-CONTRACTS.md) and the
[performance decision](M2-PERFORMANCE-OPTIONS.md); `PLAN.md` itself is unchanged.

- Request slots (208), waiting callers (128) and remote handles (64) are
  shared across all subsystems on the connection. Each subsystem additionally
  respects its server-advertised handle cap. Pipelines reserve caller
  admission as well as their 64 request slots; canceled requests and late
  handles retain admission through cleanup. The global three-worker and
  16 MiB payload limits are unchanged.
- A data-subsystem fault interrupts its assigned job while healthy siblings
  and shells continue. Already authorized queued jobs observe transport
  liveness and can obtain a fresh subsystem on the same generation; old
  browsing capabilities remain invalid. Actual transport loss interrupts
  the generation. Rejected extra subsystems are skipped for the connection,
  and remaining assignments are reused.
- Initialization remains serialized through owned cleanup if a caller is
  canceled, aborted or times out. Cancellation becomes visible promptly;
  worker/data assignments remain reserved through independent drainers.
  Those drainers retain no task/recovery-store ownership. Exit tracks weak
  transports and joins every subsystem, including data-only cleanup.
- The in-process SFTP fixture now has separate bounded request/reply paths.
  Its former serial read/write loop could deadlock both russh message queues
  under three tiny-file pipelines. The preserved failing reproduction passes
  in 0.17 seconds after the fixture correction. This finding is separate
  from the earlier visible latency outlier and does not diagnose that outlier.

All 18 required checks pass at the pool checkpoint; after the session-limit
fix and timing-independent regression synchronization, all nine affected
Rust checks pass again in the final run. Default/runtime/all-feature
Rust executions report 320/14/341 passing cases (overlapping suites), with
84/87 SFTP workflow cases and 47 SSH unit cases. The unchanged native and
frontend checks report 30 native tests, 191 UI tests, 28 terminal layout,
34 SFTP layout and 10 security checks. Generic unconfigured external tests
retain nine/default and eleven/all-feature `SKIP:` records; those returns
are not evidence of actual external execution.

The nine pool boundary tests cover one authentication/three subsystems,
fourth-assignment waiting/reuse, primary and data fault isolation, stale
browse scope refusal, late-reply assignment ownership, canceled/aborted
initialization, refusal and complete release. The data-only exit regression
proves exit joins canceled work even when the browsing primary is idle.
The new session-limit regression failed before correction: russh channel-open
refusal was incorrectly mapped to connection loss. It now reduces available
assignments and preserves two usable subsystems and sibling shells; refused
slots are not retried repeatedly. Configured WSL OpenSSH protocol and core
workflows pass, including file/editor publication, stale baseline rejection,
recursive deletion and subsystem reopening on the same transport; fixture
cleanup passes. Checks, source identity, package/measurement identities and
raw measurement attempts are indexed under
[pool evidence](baselines/M2/raw/2026-10-20-sftp-pool/README.md).
No performance acceptance is inferred from these tests.

The current production review package is `target/m2-review-9bd1cc000556`,
still version 0.1.0. Its [identity](baselines/M2/raw/2026-10-20-sftp-pool/review-r1/identity.json)
records executable SHA-256
`ADE254E66284178296C57D4C73CDE59BCA806D7FF501A788F172C2F8772B3296`
and production `--m0-check` rejection (exit 2). The measurement desktop uses
a separate package, `target/m2-benchmark-8bf76526b338`, whose dedicated entry
and executable identity pass preflight. All seven manual tooling self-tests
pass. The manual checklist points to the production review package and
extends only the affected three-transfer/cancel/tab-switch case; existing
pending editor/shell checks retain their scope.

The high-RTT three-file smoke (64 MiB per file, both directions) completes
with identical hashes, one SSH connection and two sibling shells. Product
payload admission peaks at 12,779,520 bytes and returns to zero; jobs and
pool entries also return to zero. This is one setup round, not the formal
1 GiB/three-measured-round result. Its observed ratios still indicate a gap:

| Direction | WinSCP / product data ms | Observed product/reference data ratio | WinSCP / product total ms | Observed total ratio |
| --- | --- | --- | --- | --- |
| Upload | 4,860.85 / 7,141.74 | 0.681 | 21,443.32 / 22,001.00 | 0.975 |
| Download | 9,791.59 / 20,831.06 | 0.470 | 13,825.79 / 25,782.89 | 0.536 |

The total ratio above is reference time divided by product time, so larger
is better. Small-file setup costs and TCP ramp-up may differ from full-size
rounds; these samples do not establish a formal 1 GiB failure or acceptance.

The new visible mixed smoke fails to finish its upload scenario: after
restoring the held ACK and switching back through the terminals, its last
SSH input probe times out. An opt-in diagnostic reproduction records 46
successful all-three-running checks, reaches the check before that final
probe, and hits the external 300-second deadline without a complete report.
Both attempts retain raw logs and successful network/fixture cleanup.
An intervening startup-only attempt on the just-used port is also retained;
its address-in-use error is not a product sample. Read-only TCP snapshots
from the first attempt record a live but idle interval, without proving its
cause. The later diagnostic avoids those snapshots during the input phase
and still fails to complete. No incomplete run, diagnostic timing or
partial input sequence is counted as latency acceptance.

The obsolete single-subsystem formal attempt `files-1g-high-r4` was stopped
through its owned cancellation request after approval. Completed reference
warmups and the incomplete reference round remain preserved; fixture and
network cleanup both succeeded. The full matrix must run against the new
source/executable without changing its registered sizes, rounds or gates.

Known remaining risks: full-size throughput and visible mixed-load input
latency remain unaccepted. The prior 630.7 ms upload p95 remains a failed
historical smoke; the new incomplete input runs do not supersede it. The
next engineering work is to locate the post-ACK/input stall, close the
remaining throughput gap, and run the unchanged complete file/tree/mixed
and M1 regression matrices on the resulting source. Session-channel and
subsystem refusals now have tested bounded fallback, but real servers may
still offer fewer simultaneous sessions. Affected desktop/editor/shell
acceptance, independent review and the v0.2 candidate remain outstanding.
This continuation does not reopen accepted unaffected M0/M1/A checks or
declare B/M2 complete.

The previous [browse repair review](M2-REVIEW.md) and accepted M0/M1/A-gate evidence retain their scope. The current checkpoint adds production Explorer drop/export tokens, reviewed folder plans, safe download-partial reattachment, independently drained request cancellation, and complete Bash/Zsh configuration transactions. Earlier dated sections below describe historical checkpoints; their superseded implementation gaps are resolved by this section. B/M2 is **not accepted** on the strength of automated tests.

### Confidential transfer recovery continuation, 2026-10-20

Transfer residual descriptors now use current-user Windows DPAPI for the entire bounded frame, closing the plaintext host/path metadata gap in the previous transfer ledger. Native storage exclusively holds two non-reparse, single-link files beneath a restricted directory and pins its ancestors. Additional entropy binds the directory's volume/file identity and the slot index; copied or exchanged ciphertext cannot become recovery records. The 256 KiB clear / 288 KiB sealed limits and 128-record bound remain explicit. Serialized/decrypted byte buffers are zeroized; no document or transfer body is added to the ledger, and no new dependency or IPC authority is introduced.

Decodable old plaintext is migrated only after dropping its saved authorization proof, then mirrored and synced as ciphertext. Present unchanged descriptors show `manual_inspection`, while missing/changed local files retain their specific states; neither the UI nor a stale direct backend action can promote them to recovery authority. New sealed records retain their original policy/baseline/native identity requirements. One unreadable slot falls back to a valid older frame with an incomplete-discovery warning; both unreadable slots preserve partials without scanning or adopting them. The warning survives subsequent writes/restarts. A subprocess regression exits without Rust destructors, verifies profile decryption after restart and confirms that queue tasks and picker grants are not restored. Targeted core recovery tests pass 12/12. [All 18 required checks](baselines/M2/raw/2026-10-20-transfer-protection/checks-r1/checks-all.json) pass, with workspace/runtime/all-feature Rust executions of 308/14/329, 30 native tests and 191 UI tests. [Configured OpenSSH](baselines/M2/raw/2026-10-20-transfer-protection/openssh-r1/report.json) also passes its protocol/core workflow and cleanup; generic unconfigured test returns are not counted as external execution. The [checkpoint index](baselines/M2/raw/2026-10-20-transfer-protection/README.md) records source, build and tooling evidence.

This protects stored metadata using the Windows user profile, independently of the Vault password. Loss of profile keys, a different directory identity or two damaged slots can prevent discovery; undecodable old bytes and partial data remain available for manual inspection. It is not protection from arbitrary code under the same user, queue restoration or a power-loss guarantee. The existing B-engineering-04 / B-transfer-recovery desktop cases cover same-user restart with newly created partials; no destructive crypto hand-testing is requested. Full performance/mixed-load acceptance, affected desktop/editor/shell acceptance, independent review and v0.2 candidate delivery remain open.

### Quantitative engineering continuation, 2026-10-20

The continuation under [the preregistered B engineering method](baselines/M2/raw/2026-10-20-b-engineering/method.md) adds three-worker file/tree, real TCP interruption/resume and visible terminal/resource tooling. It fixes live download retry on a replacement browse scope: Rust reauthorizes the original path under its full authenticated policy and source attributes. New persistent records retain Rust-only `sourceBaseline` and `policySha256`; legacy records missing original policy/baseline/identity remain discoverable for manual inspection. Real tree execution also exposed intermittent native publication failure at particular path lengths; the Win32 rename allocation now includes an explicit wide NUL, with native filename-alignment and concurrent tree regressions. The in-process tree reproduction passed 30 consecutive repetitions after correction.

The [evidence index](baselines/M2/raw/2026-10-20-b-engineering/README.md) separates complete engineering checks, fixed-size resume correctness and quantitative acceptance. Current Rust/native/UI checks pass, and a new production review package is available below. Loopback/high-RTT three-file and tree smoke executions retain complete hashes and resource convergence. The latest non-diagnostic visible smoke completes both directions and passes foreground/resource checks, but upload SSH input p95 is 630.7 ms against the unchanged 269.6988 ms ceiling; download is 246.7 ms. A prior intermittent stall remains unresolved. These are setup samples, not the formal three-round 1 GiB matrix. Earlier fixed high-RTT single-file measurements identify their original executable explicitly; no smoke or synthetic report substitutes for acceptance.

The earlier formal high-RTT three-file attempts do not provide a complete comparison. [r1's interruption record](baselines/M2/raw/2026-10-20-b-engineering/files-1g-high-r1/interruption.json) retains seven complete reference samples and zero product samples; its original process disappeared without a final report and the termination cause is unknown. The `files-1g-high-r2` directory has no execution report, so no execution or cleanup is claimed for it. [r3](baselines/M2/raw/2026-10-20-b-engineering/files-1g-high-r3/report.json) records owned cancellation and successful fixture/network cleanup. No complete measured comparison is accepted; original artifacts are preserved. The full comparison and visible input issue continue as engineering work under B6.

### Fixed-size bidirectional continuation evidence, 2026-10-20

[Six real 1 GiB loopback executions](baselines/M2/raw/2026-10-20-b-engineering/resume-1g-loopback-r1/report.json) now pass: upload and download each drop owned TCP sockets at requested 20%, 50% and 85% checkpoints, explicitly authenticate a new generation on the same endpoint/policy, validate the retained prefix and complete the original task. All six SHA-256 values equal `ce111325eef5d67765f53a9cb56437b685e074d84af6f7a2198530d8bcb0e7cd`. Download append offsets/counts match the actual retained sizes; upload source reads account for prefix plus remainder. Every case has two actual TCP connections, no remaining owned partial, an empty pool and zero retained payload. The earlier [six 64 MiB high-RTT cases](baselines/M2/raw/2026-10-20-b-engineering/resume-smoke-r1/report.json) remain separate smoke evidence. This closes the missing fixed-size core correctness proof; it does not accept the full performance matrix or the new desktop retry/recovery interactions.

Legacy records that lack original proof now use the bounded `manual_inspection` UI state. English and Chinese explain preservation/manual inspection and suppress continuation, cleanup and reconnect actions. No private baseline, digest or native path is added to the public DTO. Rust/contract/component tests cover missing original proof and both languages; added desktop/minimum-window metadata scenes cover the explanation's scroll reachability.

[The file workflow guide](../SFTP.md) documents browse/delete, picker and Explorer transfer, pause/retry, retained partial/export copies, editor conflict/recovery, shell setup, lock and exit behavior. The root README links it and distinguishes the M2 review implementation from accepted M1 and a future v0.2 candidate. The guide does not mark pending desktop scenarios as passed.

### Earlier engineering verification checkpoint, 2026-10-20

This preceding checkpoint adds bounded recursive remote-directory deletion to the SFTP service and Tauri/UI boundary: Rust-owned preview tokens, full-tree rescan before mutation, depth/entry/path limits, symlink unlink-without-traversal, per-entry `removed`/`failed`/`retained` results, cancel-after-current-request semantics, and exit draining. The Tauri command futures were checked for `Send` safety after the state-lock scope was tightened. Core now drains deletion, transfer and settings work concurrently under the existing 30-second exit budget; either exit route restores admission on failure. The bilingual exit dialog explains that admitted removals finish and completed deletions cannot be undone. The new Rust boundary/coordinator fixture cases, TypeScript contract/dialog tests and SFTP layout scenes are included in the full run.

The authoritative evidence is [2026-10-20 engineering completion](baselines/M2/raw/2026-10-20-engineering-completion/README.md). `Check-M2 -Group all` passed all 18 checks: workspace 299 tests, runtime 14 tests, all-features 317 tests, native 27 tests, UI 33 files/189 tests, terminal layout 28 scenes, SFTP layout 32 scenes and security 10 scenes. The real OpenSSH 9.6p1 core workflow passed and included recursive-delete verification; its report explicitly keeps `performanceAcceptance=false` and `desktopAcceptance=false`. The latest three-step review build is `target/m2-review-eb80467144bb`, with manifest SHA-256 `6B69CC7988A24FBEFE86A45B5304A2D662837B1ABAD7DAA389DF0EC3C5FFE105` and `desktop/Capstan.exe` SHA-256 `3507AD216F49AA640E91590E9CBE90D98A234E3B4278F231CA21129B683BD2D7`. The seven manual-tool self-checks passed, but they are fixture/cleanup/data-object checks only. The latest source manifest has 469 inputs and preserves the unchanged PLAN hash. B/M2 remains blocked pending affected desktop/editor/shell acceptance, the frozen full performance/mixed-resource matrix, independent review and the v0.2 candidate.

### Historical engineering completion checkpoint, 2026-10-19

This checkpoint retains the native/tree/recovery implementation details and evidence from 2026-10-19. The latest results and review-build identity are indexed at [2026-10-20 engineering completion](baselines/M2/raw/2026-10-20-engineering-completion/README.md); no commit identity is manufactured for this workspace, which has no `.git` metadata.

- **Native drop and Explorer export:** only the Rust native drop-event handler registers local paths. The custom UI event contains a 60-second, one-use receipt and logical hit-test coordinates. Rust arms the visible view first; the receipt fixes its owner, connection generation, terminal, target directory and target-lease UUID, so stale events cannot switch destinations. Enqueue revalidates the authenticated SFTP binding and native sources; files and folders are supported. Folder drops use the same reviewed tree plans as native folder pickers. Receipt metadata is bounded to 256 paths / 65,536 native path units and 128 receipts. File bodies never enter IPC. Tauri's built-in native drop metadata is not consumed as an I/O capability.
- **Export lifecycle:** preparation builds a protected current-user/SYSTEM stage and an owner-bound `ExportToken`; filename decisions happen before download. The existing three-worker/64-request engine stages and hashes files, including empty directories. The UI exposes a second, actual draggable control once ready. OLE runs on the native UI thread and permits copy only. Escape or `DoDragDrop` returning never deletes the stage. The global export entry requires explicit copy-finished consent for cleanup; exclusive file handles reject cleanup while Explorer still reads. Final root deletion revalidates the original directory identity on the same exclusive handle used to remove it. Known basename-only D/F/P manifest entries permit bounded startup discovery and explicit cleanup of recorded task partials. Unknown entries, reparse points, marker hard links and live owners are preserved. Eight stages are admitted/displayed per process, with 256 roots inspected; expiry retains bytes and recovered stages have cleanup authority only. The five-minute drag lifetime starts when readiness is observed, after the download has finished.
- **Folder/name/type decisions:** native picker and drop paths now prepare Rust-owned plans before creating destination directories. A plan binds the whole authenticated view, five-minute lifetime and exact decimal revision. Existing directories require explicit merge; type conflicts, Windows-invalid names and case collisions offer rename/skip. Renaming updates descendants; skipping a directory skips its subtree. A fresh scan invalidates stale decisions. Batch merge applies to at most 50 displayed directories, and ordinary file replacements retain their existing independent version-bound batch decisions. Single-file directory collisions also offer save-as/skip and cannot replace a directory. The legacy immediate-create folder IPC commands were removed.
- **Download recovery:** new ledger records persist the local file identity in Rust only. A fresh save picker must select the exact original target; the native adapter pins ancestors, opens the deterministic sibling exclusively, rejects reparse/hard-link/replaced files and checks confirmed-size <= actual-size <= total. Core verifies the entire existing prefix against the fresh authenticated remote source before appending and hashes the final result. Abandoned grants preserve the old partial; explicit cleanup uses that reauthorized handle and never removes the final target. Legacy records without a saved identity remain preserved and cannot be adopted. Zero-byte intents are supported, and upload intent is persisted before OPEN so a canceled/lost HANDLE reply still leaves a known recovery descriptor.
- **Cancellation and admission:** normal cancellation returns promptly while independent owned drainers retain in-flight/waiting/handle admission until a reply/close is consumed. Canceled pipelines also retain their opaque 16 MiB payload reservations through upstream convergence and CLOSE. Late OPEN handles are closed exactly once. A weak client registry and protocol task tracking let coordinated exit await these drainers without retaining finished-task SSH leases in the registry. Three file windows retain the frozen 16 control-request headroom. EOF, malformed protocol state and an undrainable 15-second protocol timeout can still quarantine SFTP and interrupt sibling file operations; this is distinct from normal per-request cancellation, and never silently authenticates or replays mutations. Native blocking I/O and full mixed-load timing remain separate limits.
- **Retry/recovery closure:** final jobs release the old client/SSH lease. Rebinding compares the original host/path/target and complete authenticated policy, allowing only a new Vault epoch. A failed admission leaves the final task revision unchanged. Only explicit successful retry reopens a final UI row; older final snapshots cannot roll it back. Current-process task IDs are excluded from startup recovery. A prefix mismatch preserves the partial and ledger, rather than discarding evidence.
- **Shell transactions:** preview/apply is single use, binding- and full-source-hash checked. Changed configuration gets a private backup, hash-verified temporary, preserved configuration mode and shared atomic publication. An unchanged install/uninstall performs no backup or rewrite. Uninstall removes only the recognized unmodified block, preserves user suffix content and keeps a necessary line separator when the original lacked a final newline. Failure/stale/equal-metadata modifications preserve the original. Tests use a non-default 0640 configuration mode and prove backups remain 0600. Real-shell hook execution is separate from maintainer acceptance of the complete UI lifecycle.


### Historical transfer-discovery checkpoint, 2026-10-19 (superseded above)

- `TransferService` now opens a bounded, two-slot recovery ledger under the Rust-selected roaming or portable data directory. Records contain task/host IDs, target metadata and the exact owned partial identity; they contain no credential, session, source bytes or download destination path in the public UI contract. Frames have a version, size bound and checksum. Startup reads only the two known slots, restores the newest valid generation and reports an incomplete scan without searching for or adopting arbitrary `.partial` files.
- Uploads write their recovery descriptor after exclusive partial creation and before file data. Interrupted uploads update the same record with the task ID path and confirmed byte count. Successful publication or verified cleanup removes it. New-format records validate that remote partial names equal `.capstan-<task UUID>.partial` beside the target, and local download partials match that task name beside the selected destination. Slot replacement and in-memory admission are ordered so a failed write does not publish an uncommitted record.
- The transfer queue shows discovered records after launch. A remote upload can resume only after a fresh, host-matched SFTP session and a fresh native source picker; the existing engine verifies the source size and remote prefix before appending. A separate explicit confirmation can remove the recorded remote upload partial after reconnect and a file-kind/size check. It never deletes the formal target. Unknown names are never scanned or removed.
- Local download partials are discovered by their exact ledger path and display as available, missing or changed after lstat/size checks. They are preserved and are not automatically opened, resumed or deleted; a new-process Windows reattachment capability that pins and verifies the original destination/partial handles remains unimplemented. A checksum detects accidental ledger damage but does not authenticate the ledger against a user able to rewrite the application data directory; the ledger grants no operation without a fresh session and explicit UI action.
- Tests cover checksum/two-slot fallback, corrupt and unknown records, exact task/parent binding, local size changes, fresh-service restart and upload continuation over the production SFTP fixture after abrupt TCP EOF. UI tests cover strict path-free DTO decoding, recovery rows, fresh picker/session binding and explicit cleanup confirmation. This closes transfer residual discovery and remote upload recovery engineering only; it does not recover queued tasks, local download partials, or grant M2 acceptance.

### Confidential remote cleanup recovery, 2026-10-19

- Editor cleanup records now survive process exit. Native staging holds two exclusive, non-reparse, single-link checkpoint files under the protected session directory. Checkpoints use current-user Windows DPAPI without machine-wide protection or UI prompts; additional entropy binds the editor UUID and basename. The versioned core frame additionally hashes its sequence and JSON, checks the session/scope/schema, and admits at most eight generated temporary paths in a 64 KiB frame (96 KiB sealed-file limit per slot). Plaintext credential material, document bytes and upload authority are absent from the journal; policy comparison uses credential IDs/revisions only. Serialized/decrypted buffers are zeroized.
- Every new remote temporary pathname is saved and synced to **both slots before OPEN**. Handle attributes and acknowledged-byte SHA-256 update the proof before publication. Removal is retired only after a checkpoint succeeds; a failed save retains the in-memory record. A torn update or damage to one acknowledged slot can fall back to the other intent, potentially retaining an unverified file for manual inspection. Late canceled writers cannot replace a newer sequence. This is process-crash recovery, not a parent-directory power-loss durability guarantee.
- Startup restores cleanup metadata and the local document without authenticating, uploading or deleting anything remotely. The cleanup command can accept an explicit current terminal/SFTP pair; core compares host, target/user/port, credential ID/revision, pinned trust, Agent and proxy policy. A new Vault epoch is allowed, while changed identity is refused. Recovered documents remain unable to bind for upload or resolve a remote overwrite. The UI requests a matching connection only after a user asks to clean up, and sends IDs rather than paths or ownership proofs.
- Added **Keep files for next start** / **保留文件供下次恢复**. It stops the edit workflow, releases remote authority and leaves the entire owned folder intact, so the user can exit with a disconnected or unreadable recovery record. Export still occurs before cleanup and can succeed while cleanup remains pending. Both unreadable slots preserve the local document, show a bilingual recovery error and prevent automatic cleanup/deletion; keeping the files remains available. Unknown editor backups remain untouched.
- Native document snapshots and cleanup now reject hard-link substitution as well as reparse points; an external linked file is neither read for upload nor deleted. Checkpoint files likewise reject hard links and remain exclusively held during the live session. External editor stdin/stdout/stderr are disconnected from the application, and a real subprocess regression verifies its output is not inherited by app logs. The journal adds only a feature to the already pinned `windows-sys` dependency, with no new package version.
- Added a real child-process exit without Rust destructors, checkpoint tear/corruption/context-swap/limit tests, failed write-ahead/removal tests, recovered cleanup on the production russh fixture with changed-credential rejection, and lost-commit-ACK recovery without upload replay. UI coverage adds recovered cleanup binding, unreadable metadata and the explicit retain action; the SFTP layout suite has 22 scenes. Actual editor and Windows interaction remain unaccepted until the new manual deltas are run.
- New evidence is indexed at [editor recovery](baselines/M2/raw/2026-10-19-editor-recovery/README.md). Previous [editor/rebind evidence](baselines/M2/raw/2026-10-19-remote-edit/README.md) is preserved as a checkpoint and is not relabeled as this source. B4's process-local-ledger implementation gap is closed; actual-editor acceptance remains. B/M2 stays blocked by the other implementation, performance and candidate requirements below.

### Remote editing and local residual recovery, 2026-10-19

- Moved editor workflow ownership out of the Tauri command file into `capstan-core::remote_edit::RemoteEditorService`. Rust owns bounded admission, a 500 ms watcher, 1000 ms stable-save detection, per-session serialization, a connection lease and independent edit lifetime. The global bilingual editor queue remains available after the source SSH tab closes. The watcher ignores unchanged committed bytes, restarts stability after a changed/missing/locked file, detects content/identity changes, and never equates a launcher process exiting with a shared editor finishing.
- Added a native `.exe` picker and a Rust-only persisted editor preference. Executable and staged document paths never enter IPC; launch uses one absolute document argv entry without a shell command. Staging uses a random UUID root, current-user/SYSTEM protected DACL, pinned non-reparse ancestors, bounded reads and an exclusive ownership handle. Added explicit editor errors to the shared bilingual metadata contract and Rust/TS fixture.
- Return uploads preserve original encoded bytes, BOM/newlines and existing permission bits. They verify SHA-256 of the current baseline and staged remote temporary, then use the transfer service's final publication gate and shared atomic rename primitive. The formal target and canonical parent are checked again after staging. Concurrent editor sessions cannot both publish an old baseline through the same application gate. External SFTP writers still have the documented final-check-to-rename race.
- Conflict decisions retain local and remote versions until the user chooses a new remote name, explicit replacement, or **keep remote and discard local**. Replacement uses the version observed at conflict, rather than accepting whatever exists at click time. Further changes require another decision. Keep-remote now removes the session only after native cleanup succeeds; failure retains a visible residual. A no-atomic-replace server can publish only to a new absent target.
- Startup discovers only bounded marked editor roots whose exclusive owner handle can be acquired. The marker contains a basename, without persisting host/remote-path authority. Unknown/forged/reparse trees and live application instances are preserved. Recovered copies support native local export or explicit discard; they never replay remote writes. Canceling the export picker keeps the UI row and dialog. Local copy publication uses the existing anchored exclusive destination; cleanup rejects a document changed since the exported snapshot.
- An open editor blocks both explicit and native window exit before settings/transfer cancellation. Exit atomically closes editor admission, and drain failure restores it. Lock-with-disconnect cancels editor work and retains the local stage. A failed editor launch also retains a local session. Runtime retry can use the retained healthy context; a new SSH generation can now be selected explicitly as described below. At this earlier checkpoint the remote ledger was process-local; the durable recovery implementation above supersedes that gap. Actual-editor acceptance and the remaining M2 gates are still required.
- Added production SSH/SFTP editor scenarios, core lifecycle/contract/stability/recovery tests, native ACL/rename-save/ownership/tamper/export tests and global UI/IPC/late-snapshot/canceled-picker regressions. The SFTP browser fixture now covers editor conflict in English and Chinese/light theme plus a 330×240 recovery entry. These are isolated metadata screenshots, not actual editor acceptance.
- Evidence and exact commands are indexed in [the remote-edit continuation](baselines/M2/raw/2026-10-19-remote-edit/README.md). `engineering-r1` preserves an all-feature failure while the explicit DACL implementation was being validated: the ACL handle needed `READ_CONTROL` as well as `WRITE_DAC`. The final native test asserts the protected two-ACE DACL. `engineering-final/rust` and `engineering-r2/ui,native` preserve the first post-fix checkpoint; `engineering-rebind` verifies the subsequent implementation below; no failed report or frozen performance reference was overwritten.

### Explicit editor reconnect and remote temporary cleanup, 2026-10-19

- `retry_remote_edit` accepts an edit ID plus an optional terminal/SFTP pair. Core resolves a current authenticated view and compares the retained host, target/port/user, credential ID/revision, trust fingerprint, Agent identity/forwarding and proxy policy. Only the Vault epoch may change after an explicit unlocked connection. Old or unrelated capabilities and changed policy are rejected. The old remote attributes/hash are retained: reconnect never authorizes overwriting a newer remote version. Startup-recovered local copies have no such remote authority.
- The global editor UI requests/focuses the host when a disconnected edit has no live view, then performs the specifically requested retry when a matching session appears. Dead SFTP views are removed from the available UI bindings. No background authentication or terminal input is replayed.
- Added a bounded ledger of eight remote temporaries per editor. Exclusive-create handle attributes and acknowledged source bytes establish cleanup evidence. Every delete rechecks canonical parent, regular-file attributes and full raw-byte SHA-256, then rechecks attributes before unlink. Publication revalidates both target and temporary under the shared commit gate. Replaced, damaged or unverified paths remain visible with their state; missing paths can retire their records. The frontend can request cleanup by edit ID only, never submit a deletion path or ownership proof.
- Failed automatic cleanup retains the edit session and prevents close/exit from silently dropping the record. Retrying on a new verified context can resume cleanup; cleanup does not adopt external content as a fresh deletion baseline. Export can publish the local copy while unresolved remote cleanup keeps the session open. The UI explains that distinction. At this earlier checkpoint the ledger was **process-local**. The durable recovery section above supersedes that limitation without adding a plaintext host/path recovery list.
- Added regressions for same-size/time temporary content replacement, denied-rename replacement, ledger exhaustion and later missing-path cleanup; new-generation retry after a Vault epoch change, changed-credential rejection, original-baseline conflict and explicit replacement; and retained cleanup evidence/blocked close across rebind. UI tests cover the deferred new-session retry and edit-ID-only cleanup. Final command logs are under `engineering-rebind` in the evidence index.

### Final editor boundary corrections, 2026-10-19

- Remote baseline/conflict inspection now reads bounded **raw bytes**, while opening a new edit and accepting local saves still require supported text. A remote writer replacing text with binary data is classified as a conflict; it does not hide the explicit preserve/save-as/replace choices behind a local encoding error. The same-size/time hash regression includes NUL content, and the new-session conflict test replaces an externally written binary version only after an explicit decision.
- Native staging and recovery reject reserved ownership/marker names with Windows case-insensitive comparison. Tests include uppercase/mixed-case names both before creating a directory and in forged recovery markers.
- This earlier checkpoint is recorded under `engineering-hardening` (166 UI tests and 20 layout scenes); current recovery verification is separate below.

### Recursive folders and retry guidance, 2026-10-19

- Folder upload now retains the picker-selected root name, enumerates bounded regular directories separately from files, creates the remote directory tree parent-first, and then queues each file with an owner-bound native source. Empty directories are created even when the selected folder contains no files. Existing remote directories are accepted; an existing non-directory aborts before file tasks are queued.
- Folder download now collects a bounded Rust-owned remote tree from the selected directory entry, rejects symlinks/unsupported types instead of following them, asks for one native destination folder, creates every remote directory locally (including empty directories), and queues each regular file through the existing hash/conflict/commit pipeline. Remote and local paths remain outside the WebView contract.
- An interrupted retry with no matching active SFTP binding now offers an actionable reconnect/open-session prompt. The host tab is focused or created through the existing workspace path. The requested retry continues when that host receives a fresh SFTP session; uploads still require a new native picker. This is continuation of a user-requested task, without automatic SSH authentication or terminal-input replay.
- Added native Windows tests for directory enumeration, empty-directory preservation and anchored nested destinations, a bounded selection test for folder download, and a queue-level UI test for retry guidance.

### Folder batch conflict decisions, 2026-10-19

- Folder upload/download now performs one Rust-side preflight for every destination version and creates a random batch ID for that picker selection. The UI can apply **replace batch** or **skip batch** to ordinary files that existed unchanged at preflight; the batch metadata is bounded and never becomes IPC authority.
- A batch grant is bound to the exact selected tree and rejects cross-root, cross-batch, duplicate or forged members. Single-file transfers, remote editing and retries keep their existing one-file decisions and never inherit a folder policy.
- A target that appears, disappears, changes type/attributes, or changes again before publication loses batch eligibility and must be decided per file. Servers without the advertised safe atomic replacement capability refuse batch overwrite; skip or explicit save-as remains available.
- Added Rust workflow cases for upload/download replace and skip, empty-root preservation, target appearance/disappearance, final identity rechecks, retry policy isolation, grant binding and non-atomic servers. UI contracts/queue tests cover bilingual batch controls, version boundaries and the per-file fallback. The full directory/name/type conflict workflow and production GUI acceptance remain open.
- Evidence for this increment is indexed at [folder conflicts](baselines/M2/raw/2026-10-19-folder-conflicts/README.md); the current source/evidence inventory is [manifest](baselines/M2/raw/2026-10-19-folder-conflicts/manifest.json).

### Bidirectional partial continuation and explicit retry, 2026-10-19

- Added `SftpClient::open_existing_file` and `PipelinedFile::seek_to` so an upload can reopen its owned remote `.capstan-*.partial` and a download can seek its verified local partial before issuing new data requests. Both directions validate the completed prefix against the current source/remote bytes and reject mismatches with `resume_mismatch`/`target_conflict` semantics before appending.
- Interrupted work retains only the partial state needed for an explicit retry. The old upload source handle is released on disconnect; a retry must provide a fresh picker and a newly attached SFTP session. A matching new selection reuses the retained partial, while changed source bytes, a replaced remote partial, an invalid partial name or an inconsistent local size fails closed. Successful commit removes the partial; uncertain cleanup remains visible.
- Added local partial-size support for the Windows destination and kept the resume path behind the existing owner-bound token/session checks. No path or file body crosses IPC. The TCP EOF integration test now drops the original transport, opens a new connection and picker, verifies the prefix, completes the upload, checks the final hash and confirms that the partial is cleaned up.
- At this earlier continuation checkpoint, startup residual discovery had not yet been added. The later recovery increment above provides bounded discovery and explicit upload recovery; this checkpoint still does not claim a complete 1 GiB high-RTT product matrix, Explorer drag/export, full directory conflict decisions, or acceptance of the replacement-session/picker UI.

### Earlier folder/retry checkpoint, 2026-10-19

- `cargo fmt --all`, `cargo check --workspace`, `cargo clippy -p capstan-core --all-targets -- -D warnings`, and `cargo clippy -p capstan-app --features tauri-runtime --all-targets -- -D warnings` passed.
- `cargo test --workspace` passed: application/core/SSH/PTY/terminal/vault unit and integration suites completed with zero failures, including 32 SFTP workflow cases and the abrupt TCP EOF continuation case.
- `cargo test --manifest-path vendor/capstan-native-windows/Cargo.toml` passed: 11 native tests, including nested destination and empty-directory coverage.
- `pnpm -C ui typecheck` passed and `pnpm -C ui test -- --run` passed: 24 test files, 153 tests.
- `docs/PLAN.md` remains unchanged with SHA-256 `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`.

### Panel preferences and coordinated exit, 2026-10-18

- Added framework-independent `Settings` at the Rust-selected application data root. Only `{dock, size, collapsed}` is added to `settings.json`; tab IDs, hosts, paths, aliases, file bodies and credentials are not persisted by this feature. A bounded 16 KiB versioned document preserves unrelated settings and refuses malformed, oversized, unsupported-version or unsafe native paths without overwriting them. Held Windows handles, target identity rechecks and a synced same-directory temporary provide atomic publication.
- Added thin main-window load/save commands with a runtime session token and exact-u64 sequence. At most 32 admitted writes are retained; older requests cannot overwrite the latest admitted choice. Blocking I/O retains its admission until completion even if the caller drops its future. Existing SSH tabs retain independent layouts; new tabs and restarts inherit the last saved default. Late initialization and old save responses cannot overwrite a newer session choice.
- Localized load/save recovery keeps browsing and current-session layout usable. Collapsed panels retain an accessible warning; retry reloads the storage baseline while preserving a dirty local choice. Exit stops new settings writes and drains them concurrently with transfer cancellation/publication under the existing 30-second bound. A timeout keeps the window open and restores both services' admission; already canceled jobs are not replayed. Settings writes can still outlast the timeout inside native blocking I/O.
- [Preferences evidence](baselines/M2/raw/2026-10-18-panel-preferences/README.md) records five new Rust tests and eight additional UI cases, full checks, localized failure/layout scenes, real OpenSSH regression and a new production review build. The first runtime-only build failure exposed use of an optional benchmark JSON dependency; a typed serializable event fixes it without adding a dependency. The failed logs remain preserved. These additions have not been included in the earlier maintainer acceptance.

### Bash/Zsh managed-block prerequisite, 2026-10-18

- Added a Rust-only, 64 KiB UTF-8 configuration planner with a versioned application marker. Repeated installation is a no-op; uninstall removes only a fully recognized generated block and preserves surrounding bytes. Unknown/edited/duplicate markers, reserved-function collisions, invalid encoding/control characters and mismatched shell/authority fail closed. If installation inserted a missing line break, removing it additionally requires the original prefix hash; an ambiguous boundary requires manual repair instead of changing unrelated bytes. This planner performs no remote or local configuration I/O.
- Generated hooks use only validated authenticated-target authority and byte-wise URI encoding, bound hints to 8192 bytes, skip nonabsolute/oversized paths and run only interactively. Bash preserves scalar/indexed `PROMPT_COMMAND` and the original command status. Zsh preserves `precmd_functions` and returns success so later hooks are not suppressed; its hook dispatcher supplies the original status independently, as documented by the [Zsh manual](https://zsh.sourceforge.io/Doc/Release/Functions.html#Hook-Functions). Read-only/unsupported hook variables remain untouched. Repeated sourcing does not duplicate registration.
- [Shell evidence](baselines/M2/raw/2026-10-18-shell-integration/README.md) has four actual planner tests, one separately identified conditional fixture exporter, and **16 actual isolated WSL PTY cases** across Bash 5.2.21 and Zsh 5.9, login/nonlogin, existing/empty/readonly/nounset hooks, repeat source, original exit status, Unicode/space/percent/quote path encoding, root and rejected hints. Temporary homes are cleaned; Zsh was downloaded as an exact Ubuntu package and extracted only under the workspace, without installing it or changing existing shell configuration.
- This is the M2-10 planner and shell-fixture prerequisite. The application now wires authenticated file selection, current-content revalidation, backup, permission-preserving atomic remote apply, preview-token lifecycle and UI confirmation; complete installer lifecycle/remote install-uninstall acceptance and affected manual evidence remain required. B5 stays blocked.

### Production file pipeline and measurements, 2026-10-18

- Vendored the exact Apache-2.0 `russh-sftp` 3.0.1 source, preserving its original archive hash, manifests, license and VCS identity. A narrow [patch](../../vendor/russh-sftp/CAPSTAN_PATCH.md) reuses its existing File pipeline on an already authenticated facade handle, exposes contiguous successful WRITE acknowledgements, drains issued requests before CLOSE, fills only short-read gaps, preserves typed STATUS errors, and refuses request-ID wrap. It adds no second protocol dispatcher or authentication path.
- Production upload, download and complete remote hash readback now use up to 64 requests of 32 KiB each. Three file windows leave 16 control slots inside the frozen 208-slot bound. A whole window is charged before use and held through convergence; upload releases its first window before acquiring the verification window. Deterministic peers cover a completely unacknowledged window, reversed replies, a short-read gap, failed-ACK holes and healthy sibling/control progress while another file stalls. The core file-size-doubling peak is now 4,259,840 admitted payload bytes, returning to zero for both tested sizes.
- The real **1 GiB loopback** product comparison passes: upload/download data-phase ratios to WinSCP are **3.9631 / 1.7239**; end-to-end ratios including engine integrity verification and commit are **2.5042 / 1.6137**. The real **64 MiB controlled-high-RTT** comparison also passes: data-phase ratios **1.0285 / 1.1091**, end-to-end **0.9934 / 1.0148**. All four directions meet both the unchanged 80% threshold and frozen absolute data-throughput floors. Each scenario retains 16 full-hash samples, fresh connections on both clients, identical product/WinSCP binary identities and fixed algorithms. Fixture/network cleanup and temporary awake-request release succeed.
- [Pipeline evidence](baselines/M2/raw/2026-10-18-pipeline/README.md) retains the invalid suspend-crossing first run and the second run's process-exit reporting failure. A bounded owned-process launcher and its two self-checks fix the latter; the successful third high-RTT run and low-RTT run remain separate. Measurements bind the saved pipeline candidate source and executable; the later cwd changes have separate source/test evidence. These single-task results do not complete the 1 GiB high-RTT, three-worker, small-tree, mixed-terminal, resource or candidate acceptance matrix.

### OSC 7 directory following, 2026-10-18

- xterm's OSC 7 callback forwards parsed hints without waiting for IPC. A bounded per-terminal source keeps only the latest pending value, limits it to 8192 UTF-8 bytes and emits at most every 250 ms. Source identity is installed before pre-start terminal output is released; closing it cancels timers/subscriptions and ignores late callbacks.
- Rust derives the authority from the authenticated pool lease and rechecks window, terminal, SFTP view and connection generation on every update. Monotonic exact-u64 sequence handling prevents old hints replacing newer ones. Cross-host, malformed and oversized URI cases preserve the last valid state. Up to eight explicitly bound aliases are terminal-scoped; changing aliases revalidates or revokes stored hints without granting file/write/execute authority.
- Added bilingual per-tab Follow and explicit host-alias controls. Hidden/collapsed tabs retain hints without browsing; manual directory changes disable Follow; re-enabling uses the newest valid hint. Picker/confirmation/mutation scopes are held until the interaction ends. Validation is serialized and its backlog coalesced; stale attachment/configuration responses are ignored. Failed directory reads preserve the current list and expose the existing actionable error.
- [Cwd evidence](baselines/M2/raw/2026-10-18-cwd/README.md) covers hostile/shared URI cases, binding/isolation, alias revocation, flood coalescing, late responses, StrictMode cleanup, native-picker scope and actual split binary OSC parsing/ACK. Shell integration installation is still required; the follow feature can consume an existing correctly bound OSC 7 producer without installing anything automatically.

### A1 engineering completion, 2026-10-18

- Added an isolated high-RTT WSL fixture: netem on a newly owned namespace/veth pair and loopback relay, with runtime-generated pinned SSH credentials. Existing host/default qdiscs and SSH servers are untouched. Actual ICMP and authenticated SSH nonce-echo measurements are recorded separately from connection preparation. Setup failures and cleanup results remain archived.
- Measured the selected upstream file pipeline at 16 and 64 requests. The 16-request configuration misses both high-RTT targets. The formal 64-request references use the same executable hashes, fixed algorithms, no compression, and fresh data SSH connections on both clients for every sample. One warmup plus three measured rounds in each direction/client yields 32 formal complete-hash samples across 1 GiB loopback and 64 MiB high RTT. Compiler processes are excluded during samples.
- Both formal comparisons pass the unchanged 80% target. Loopback upload/download throughput ratios are **5.1893 / 1.6614**; controlled-high-RTT ratios are **0.9309 / 1.3082**. High RTT SSH echo p95 is 109.8494 ms; the injected path has 50 ms netem on each owned veth end. The raw samples include preparation, transfer, verification and total times. These are the earlier A1 library feasibility measurements; production results are recorded separately above.
- Froze [performance-policy.json](baselines/M2/performance-policy.json), SHA-256 `59FBFCAD8AB0995D9B2342E6691A3C25C9159B0420526D168870D3EFAA296F15`, before product candidate measurements. Per-direction minimum throughput comes only from WinSCP medians; existing 80%/1.5× and all M1 limits remain unchanged. New mixed-load p95 ceilings are 156.9 ms loopback and 269.6988 ms high RTT; no product mixed-load result is claimed. The freeze rejects invalid/failed references and repeated overwrites, with hashes proving the policy was preserved on rejection.
- Froze 32 KiB requests, a 64-request per-file maximum, three workers, 208 protocol slots (192 file + 16 control), 128 waiting callers, and 16 MiB of owned transfer payload admission. Added core/SSH policy conformance tests. Reserve before reading/allocating, charge two copies, and retain permits through native blocking work. The instrumented 512 KiB/1 MiB upload/download case has identical 64 KiB payload peaks and zero retained admission after completion; the in-process peer's own file store and whole-process RSS are excluded from this meter.
- Added core OSC 7 authority/URI validation and the shared hostile [fixture](fixtures/M2-cwd.json), including explicit aliases, bracketed IPv6, cross-host rejection, UTF-8/percent/control/length validation and exactly one decode. This completes the A input-validation contract; xterm, per-terminal sequence/navigation and shell integration remain B work. Editor source/encoding/size, executable-ID and stable-save boundaries are frozen in [M2-CONTRACTS](M2-CONTRACTS.md), without claiming an editor implementation.

The [A1 evidence index](baselines/M2/raw/2026-10-18-a1/README.md) preserves failed 16-request results, earlier connection-method asymmetry, corrected fair references, the exact probe source used for measurement, the first failed engineering run and final checks. [Its manifest](baselines/M2/raw/2026-10-18-a1/manifest.json) binds selected before/current source identities; older manifests are immutable. A2–A6 retain their previous implementation/manual scope. No new manual item is needed for this backend-only prerequisite work; future product changes need their affected regressions.

### Maintainer manual acceptance, 2026-10-18

The maintainer replied **“测试全部通过”** after the [delivered manual checklist](M2-MANUAL.md). A-01–A-08, four-file complete hashes, and the separate Explorer file/folder/multiple-item copy/cancel feasibility probe are accepted as **maintainer-reported pass**. A2–A6 are accepted for the shared-connection, browse/nonrecursive, native-picker single-file and visible Windows/OpenSSH scope. Ordinary lock/exit and terminal-input correctness checks are closed; quantitative mixed-load measurement remains B6 work. No repeat of unchanged accepted manual items is requested.

The [feedback provenance](baselines/M2/raw/2026-10-18-maintainer/README.md) and [structured record](baselines/M2/raw/2026-10-18-maintainer/feedback.json) preserve the exact reply and scope. No separate per-case trace, interactive report, tested executable hash, exact platform/display matrix or cleanup result was supplied. Last-delivered bundle identity is a reference, not a fabricated independently matched manual identity. Historical core/setup reports retain their original acceptance flags. That documentation-only update has its own [manifest](baselines/M2/raw/2026-10-18-maintainer/manifest.json); the current A1 code changes have separate evidence above.

A1 engineering prerequisites are closed. Production pipeline, cwd following, panel preferences, core partial continuation, recursive folders, regular-file folder batch decisions, the single-file retry/session path and transfer-partial startup discovery with explicit remote-upload recovery now have engineering evidence; product drag/export, incompatible directory/name/type conflict decisions, local-download reattachment, complete shell-installation acceptance, full performance/regression and candidate implementation remain open. Editor monitoring/conflicts/local recovery and explicit reconnect retry are implemented with automated evidence, pending actual-editor acceptance. The earlier human feedback does not accept the new or future workflows.

### A-gate readiness continuation, 2026-10-18

- Honor the server's advertised `limits@openssh.com` handle budget, capped at 64. Exhaustion rejects the extra OPEN locally without closing a healthy shared subsystem; closing an existing handle permits a later OPEN. Three new protocol capability tests cover this behavior, exact extension version `1`, packet/read/write limits, zero/unlimited limits, u64 maxima and invalid tiny packets.
- Added an actual TCP proxy EOF while a WRITE is stalled. Unlike an orderly SSH disconnect, the peer sockets disappear without a disconnect message. The production job becomes interrupted, releases its Windows source handle, retains uncertain cleanup, publishes no target, does not reauthenticate/replay, and leaves an independent transport usable.
- Added a reproducible production-feature review build and isolated manual fixture. `Build-M2Review.ps1` packages the desktop, verified ConPTY runtime/licenses and two separate test helpers with SHA-256 inventory; `Prepare-M2Manual.ps1` verifies that inventory, prepares a new encrypted Vault and pinned loopback hosts through Rust, then opens the review desktop. Credentials are runtime-generated and removed on Stop. The normal application gains no fixture command or arbitrary file access.
- Added four deterministic manual input files (empty, text, Chinese/space name and 128 MiB binary), full hash verification, cleanup and negative tool checks. The separate bilingual Win32 drag probe creates real Shell file/directory/multiple-item data objects and records copy/cancel events. Its automated self-test creates objects only; actual Explorer copy/cancel is now accepted by the maintainer report above. Staging is retained because OLE return alone does not prove asynchronous copying is finished.
- Replaced uptime-dependent activity tests with an advanced Tokio clock. They no longer subtract 31 minutes from a Windows Instant that may be younger than that after startup. Production uses the same monotonic clock through Tokio; saved lock policy and cleanup routing are unchanged.
- Prepared the executable Chinese [manual checklist](M2-MANUAL.md). It separates already implemented Windows interactions from missing B features. The review package remains version 0.1.0 and is not a v0.2 candidate.

### Lifecycle continuation, 2026-10-18

- Fixed jobs stuck in `queued` or `awaiting_conflict` after the shared SFTP closes. A read-only close notification now wakes worker admission, conflict waits and publication-gate waits. Closed jobs enter `interrupted` with `connection_lost`, release native sources/worker permits, clear stale decisions and retain cleanup uncertainty without replaying mutations. Already admitted native commits still finish and report their actual result; blocking native I/O is not abandoned.
- Added transport liveness independently of startup cancellation. Remote teardown marks the pooled connection closed without changing host-trust, authentication or cancellation error semantics. The production Vault connection path now retires a known-dead pool entry and its aliases on an explicit new acquisition. Concurrent explicit opens still share one authentication; retired-generation leases cannot decrement the replacement's counters. This does not automatically reconnect, rebind old tasks or replay shell input.
- Bound the 30-second pool idle deadline to the last lease release, including when the cleanup task is scheduled late. A production SSH/SFTP integration test retains only the SFTP lease, drops it, and verifies the real transport's reclamation with a simulated clock (one timer-wheel tick of allowance). It is not a wall-clock desktop timing measurement.
- Added eight production pool/service lifecycle scenarios: rejected subsystem and invalid-version initialization, explicit subsystem recreation with old capabilities rejected, last-SFTP-lease idle close, active/queued peer disconnect, queued/conflict close, explicit new transport generation and alias retirement, admitted native publication versus a waiting publication plus exit drain, and window cancellation clearing unused grants. Added one delayed-idle-reaper regression in the SSH pool tests. The fixture now counts active subsystems and reclaims handle counters even when a peer task is canceled.
- Extended the real OpenSSH core workflow to use checked pool acquisition, reject the closed view and explicitly reopen SFTP on the same authenticated transport. The prior implementation report is preserved in [the lifecycle before snapshot](baselines/M2/raw/2026-10-18-lifecycle/before/docs/agent/M2-B.md).

### Single-file foundation, 2026-10-17

- Added framework-independent `capstan-core::transfer`: single-use native grants, a bounded global queue with three workers, state/progress/speed/ETA snapshots, revision-bound replace/save-as/skip decisions and cancellation/exit draining. Tasks retain their own authenticated SSH connection lease when the originating tab closes. Final target checks and publication are serialized across app jobs; temporary staging remains concurrent.
- Added Windows native open/save selection and bounded regular-file I/O. Local paths never enter IPC. Held ancestor handles reject reparse-point traversal and deny directory rename; sources deny concurrent writes/deletion. Downloads use an exclusive same-directory temporary, sync/readback, precise destination identity checks and rename/delete by the held handle. Windows traversal, ADS, reserved names, UNC/device paths and unsafe aliases fail closed.
- Implemented Rust-only streaming uploads/downloads with at most 32 KiB sequential requests. Uploads use exclusive remote random temporaries, FSTAT, advertised fsync, explicit close, complete SHA-256 readback and metadata rechecks. Downloads check lstat/FSTAT/EOF, sync and hash the local temporary before publication. Existing targets survive injected write, sync, hash and commit failures. Upload replacement creates mode 0600; permission/timestamp preservation is not automatic.
- Hardened failed-upload cleanup: capture the last attributes through the owned remote handle, then compare lstat before deletion. Changed or uncheckable temporaries are retained with `cleanupRequired`; a known rejected exclusive OPEN no longer reports a nonexistent residual. Regression tests preserve both the formal target and a replaced temporary. SFTP v3 still lacks atomic compare-and-delete.
- Added `posix-rename@openssh.com`, remote FSTAT/fsync and safe replacement capability checks to the SSH facade. Unsupported replacement offers save-as/skip. A lost commit ACK is never replayed: read-only reconciliation on a new SFTP subsystem of the same authenticated transport proves the formal target's full hash/attributes and temporary disappearance, or retains an explicit uncertain cleanup result.
- Added thin Tauri picker/grant/enqueue/snapshot/cancel/conflict commands, native close coordination and lock-policy wiring. No generic fs/shell/plugin, arbitrary executable command, file body or new credential path was added. Attach now promptly handles terminal revocation during initialization.
- Added the bilingual panel upload/download actions and global queue, stable task rows, conflict and exit dialogs, exact u64 decoding and late-response/token release protection. Queue state remains accessible after closing the source tab. Use Refresh after a completed upload to reload the remote list.
- Extended the actual in-process russh fixture, runtime-generated test data/keys, Rust/TS shared transfer fixture, real OpenSSH runner and isolated browser layout checks. The SFTP layout check is part of `scripts/Check-M2.ps1 -Group ui`.

## Verification and evidence

The current [18 required checks](baselines/M2/raw/2026-10-20-transfer-protection/checks-r1/checks-all.json) all exit 0. Workspace/runtime/all-feature counts are **308/14/329** (overlapping executions), with **33 UI files/191 tests** and **30 native tests**. The same group passes all 34 SFTP scenes, 28 terminal layout checks and 10 security checks. The earlier B-engineering reports retain their initial fmt failure and corrected passing rerun; earlier checkpoints retain their original results.

The [production review build](baselines/M2/raw/2026-10-20-transfer-protection/review-r1/report.json) passes all three steps. Its package is `target/m2-review-0744cb9bd195`, with manifest SHA-256 `D71C21B4F665DF9923E5627BD1CF6AAA06A25FB0D3D2C9AF1E3EDAB7B1E48DB0` and desktop SHA-256 `767EF39A98C6CB69B4273BBFEDCA4AAF4408163A2B9CB53D5E1728CA3F0BF566`. [Production preflight](baselines/M2/raw/2026-10-20-transfer-protection/production-identity.json) confirms that it rejects the measurement entry (exit 2). It is a **0.1.0 review build**, not a v0.2 candidate. The separate opt-in measurement build is never substituted for this package. Current [manual tooling](baselines/M2/raw/2026-10-20-transfer-protection/manual-tools-r1/report.json) passes 7/7 checks; earlier [workflow tooling](baselines/M2/raw/2026-10-20-b-engineering/workflow-tools-cancel-final/report.json) passes nine helper checks and 15 nested synthetic verifier cases, including owned-process cancellation. The [current selected-source manifest](baselines/M2/raw/2026-10-20-transfer-protection/source-manifest.json) supplements the original [pre-matrix snapshot](baselines/M2/raw/2026-10-20-b-engineering/source-before-three-worker-formal.json), which retains 488 selected inputs and the unchanged PLAN hash. There is no `.git` metadata; selected hashes are not commit diffs or an exhaustive reproducible-build claim.

The final nonvisual UTF-8 decoder correction additionally passes [types/tests/build rechecks](baselines/M2/raw/2026-10-18-cwd/ui-contract-hardening/checks.json). [Pipeline](baselines/M2/raw/2026-10-18-pipeline/manifest.json) and [cwd](baselines/M2/raw/2026-10-18-cwd/manifest.json) manifests bind 80/38 selected current sources, saved/reconstructed baseline distinctions, 32 product samples, checks, review/build/smoke and unchanged policy/PLAN hashes. They retain the exact earlier measured candidate source separately from later cwd edits. No missing before copy is represented as a Git baseline. The newer [preferences manifest](baselines/M2/raw/2026-10-18-panel-preferences/manifest.json) separately binds its changes and preserved historical manifests. The subsequent [shell checkpoint](baselines/M2/raw/2026-10-18-shell-integration/manifest.json) binds the planner, exact generated scripts and real PTY evidence without rewriting previous identities.

| Check | Result / limit |
| --- | --- |
| Workspace fmt; Clippy default and all features with `-D warnings` | Pass; all 18 required checks exit 0 in the current report; the earlier initial fmt failure remains archived |
| Workspace / runtime / all-feature Rust tests | Current counts are 308 / 14 / 329 passed, zero failures; overlapping executions, not unique totals. Opt-in external/shell/performance cases still early-return in the generic group. Retry/persisted-policy/manual-inspection, encrypted transfer discovery, native publication and qualitative mixed-load regressions are included |
| Production in-process SFTP workflow fixture | Default workspace and all-feature runs pass; the focused recursive-delete suite is 6/6. Configured OpenSSH is a separate execution and is reported below |
| Actual Bash/Zsh hooks | [16/16 isolated real PTY cases](baselines/M2/raw/2026-10-18-shell-integration/real-r3/shells/report.json), temporary homes cleaned; the remote preview/apply path is wired, while complete installer lifecycle/manual acceptance remains open |
| SFTP excluded crate fmt, Clippy and tests | All three commands pass; 3 tests, including checked request-ID exhaustion |
| Native excluded crate fmt, Clippy and tests | All three pass; 30 tests, including rename path termination/alignment, editor/transfer DPAPI slots, torn update, context swap, checkpoint/document hard links, ownership and export data-object coverage. Interactive dragging remains separate |
| Review build / production entry | [Current review build](baselines/M2/raw/2026-10-20-transfer-protection/review-r1/report.json) passes three build/helper steps and includes current recovery/native fixes. Production rejects the measurement entry. No Vault unlock, installation/upgrade or actual workflow acceptance is inferred |
| Manual tooling | Current [7/7 checks](baselines/M2/raw/2026-10-20-transfer-protection/manual-tools-r1/report.json) pass: isolated fixture/encrypted Vault preparation and cleanup, missing downloads, local-copy-only verifier success, corruption rejection, idempotent stop, outside-root rejection and native data-object setup. Expected failures have nonzero exits and retained reports; local copies are not SFTP evidence |
| Tauri runtime check | Pass |
| Cargo deny and license audit | Pass; duplicate-version warnings remain; 398 Cargo and 249 frontend packages reviewed |
| UI typecheck/tests | Pass; 33 files / 191 tests, including bilingual manual-inspection recovery, metadata privacy and recursive-delete coverage |
| UI production build | Pass; Vite's unchanged 500 kB chunk warning remains in the build log |
| Existing terminal layout / security | 28/28 layout checks across scale 1 and 1.25; 10/10 security checks |
| SFTP layout regression | 34 metadata browser scenes pass, including scroll-reachable manual-inspection explanation, recursive-delete results, folder conflict, light/Chinese and compact variants. Actual editor/Explorer behavior is still manual |
| Prior Impeccable detector / finish review | One detector run returned `[]`; [review](baselines/M2/raw/2026-10-17-transfers/ui-finish-review.md) found F1 responsive clipping, [verdict](baselines/M2/raw/2026-10-17-transfers/ui-finish-verdict.md) scores F1 resolved (`ship` for that fix list only). This is rendering evidence, not maintainer acceptance |
| Current cwd/alias finish review | One detector run returned `[]`; [fresh full scoped review](baselines/M2/raw/2026-10-18-cwd/ui-finish-review.md) returns `ship`, no material fixes, after host-specific alias recovery was added. [Design evidence](baselines/M2/raw/2026-10-18-cwd/ui-design-documentation.md) preserves inherited seed/sidecar/reference limitations; no native/M2 acceptance is inferred |
| Current preferences finish review | One detector run returned `[]`; [fresh review](baselines/M2/raw/2026-10-18-panel-preferences/ui-finish-review.md) found one Chinese read-error action mismatch, [verdict](baselines/M2/raw/2026-10-18-panel-preferences/ui-finish-verdict.md) resolves that named fix (`ship` at fix scope). [Design evidence](baselines/M2/raw/2026-10-18-panel-preferences/ui-design-documentation.md) retains the incumbent and inherited limits; native restart/error interaction remains pending |

The in-process transfer cases exercise full hashes/empty files/short reads, stale and changing conflicts, missing extensions, remote write/commit/hash and local write/sync/commit errors, temporary replacement before cleanup, known OPEN rejection, one-use owner-bound grants, three-worker admission and queued/in-flight cancellation, terminal-independent leases, exit draining, revocation during attach, same-target publication and lost-ACK read-only reconciliation. Native tests additionally reject a replaced destination whose size and mtime match but file identity differs. Fault injection is not a claim that a real disk was filled or every Windows filter/filesystem was exercised.

The first strengthened responsive check failed the Chinese compact row bounds by 1.5 px; [failed group](baselines/M2/raw/2026-10-17-transfers/ui-responsive/checks-ui.json) and its captures remain. A 112 px protected table region and scrollable secondary controls pass the final six scenes in [the layout report](baselines/M2/raw/2026-10-17T13-27-42-332Z-sftp-layout/report.json). [Design documentation](baselines/M2/raw/2026-10-17-transfers/ui-design-documentation.md) records the comparison with the incumbent system; no new visual world or shipping raster asset was introduced.

Workspace logs explicitly retain unconfigured `CAPSTAN_TEST_SSHD`, `CAPSTAN_TEST_WSL`, `CAPSTAN_TEST_CONPTY_DIR`, `CAPSTAN_TEST_SFTP_PERFORMANCE` and `CAPSTAN_TEST_TRANSFER_PERFORMANCE` and `CAPSTAN_TEST_SHELL_INTEGRATION` cases. Reported `ok` from those early-return tests is not external execution. Current M0/M1 acceptance remains based on its existing evidence, not these skips. Cargo deny used the locally available advisory database at the recorded check time; no stronger database-freshness claim is made.

The latest [OpenSSH report](baselines/M2/raw/2026-10-20-transfer-protection/openssh-r1/report.json) and [core log](baselines/M2/raw/2026-10-20-transfer-protection/openssh-r1/core-workflow.txt) record an actual OpenSSH 9.6p1 Ubuntu-3ubuntu13.20 run after the current fixes. The protocol and core workflow exit 0, clean the fixture, and include an isolated recursive directory deletion with nested entries and a final not-found check. `performanceAcceptance` and `desktopAcceptance` remain false. This run does not launch Explorer or an external editor. The separate six-case 1 GiB resume proof is recorded above; earlier OpenSSH evidence retains its original dated identity.

The earlier formal serial [loopback reference](baselines/M2/raw/2026-10-18-a1/low-r1/library-performance/report.json) and [high-RTT reference](baselines/M2/raw/2026-10-18-a1/high-rtt-r5/library-performance/report.json) measured the upstream 64-request pipeline before product integration. Both use fixed matching executables, verified Curve25519/SHA-256 + AES-256-GCM, no compression and fresh data connections per sample. One warmup plus three measured rounds per direction/client, complete hashes and pre-library WinSCP thresholds are retained. No compiler runs during samples. The earlier readiness reference and A1 r3/r4 retain their connection-method limitations and are not formal freeze inputs. Current product comparisons are separate records above and do not replace these frozen inputs.

All 16 loopback hashes match `CE111325EEF5D67765F53A9CB56437B685E074D84AF6F7A2198530D8BCB0E7CD`; all 16 high-RTT hashes match `618A0837F7DE43C18EEE4C66C8920E7D83B9599FA352E44F00511853879E5531`. With WinSCP 6.5.6.0, transfer-phase medians are:

| Scenario / direction | WinSCP median | Library probe median | Library / WinSCP throughput | 80% library feasibility |
| --- | --- | --- | --- | --- |
| Loopback 1 GiB upload | 42,259.18 ms | 8,143.48 ms | 5.1893 | Pass |
| Loopback 1 GiB download | 40,679.66 ms | 24,484.95 ms | 1.6614 | Pass |
| High RTT 64 MiB upload | 4,897.03 ms | 5,260.62 ms | 0.9309 | Pass |
| High RTT 64 MiB download | 9,769.09 ms | 7,467.34 ms | 1.3082 | Pass |

Preparation, complete-hash verification and total time remain in every [loopback](baselines/M2/raw/2026-10-18-a1/low-r1/library-performance/samples.json) and [high-RTT](baselines/M2/raw/2026-10-18-a1/high-rtt-r5/library-performance/samples.json) sample; no product end-to-end speedup follows from transfer-phase comparisons. Full production 1 GiB/high-RTT/three-worker/tree/mixed-terminal measurements remain B6 work under the frozen policy.

Earlier transfer attempts remain under `2026-10-17-transfers/openssh/` and `openssh-r2/` (a test compile lifetime error and public-key-comment constructor mismatch), followed by successful r3/r4/r5. These setup fixes did not bypass host trust. Every successful report has `performanceAcceptance: false` and `desktopAcceptance: false`. Shell co-existence does not establish mixed-load input latency.

Lifecycle failures remain archived too: the [pre-fix disconnect test](baselines/M2/raw/2026-10-18-lifecycle/before-disconnect-regression.txt) timed out; the [saved old idle function](baselines/M2/raw/2026-10-18-lifecycle/before-idle-regression.txt) fails the delayed-reaper regression. The first integration idle attempts needed timer scheduling and one tick of timer-wheel allowance. The [first SSH library run](baselines/M2/raw/2026-10-18-lifecycle/ssh-unit-tests.txt) caught three trust-error classification regressions from coupling transport liveness to startup cancellation; the independent liveness token restores all 32 library cases in [the focused rerun](baselines/M2/raw/2026-10-18-lifecycle/ssh-unit-fixed.txt) and the final workspace checks. Earlier failed exits are not rewritten as passing.

The [initial engineering group](baselines/M2/raw/2026-10-18-lifecycle/rust/checks-rust.json) also preserves the configured OpenSSH test's pool-type inference compile failure after checked acquisition was added. An explicit `ConnectionPool<PasswordConnection>` annotation fixes the fixture; all nine commands pass in the final group linked above.

Readiness retains its failed/incomplete runs as well: the first native probe missed Win32 imports, its first Shell setup exposed mixed path separators, the initial helper test accidentally constructed a traversal-containing path that validation correctly rejects, and the first PowerShell verifier wrapped a JSON array one level too deeply. Final setup and verifier checks cover success, missing and corrupted downloads plus cleanup rejection. Interrupted engineering runs left corrupt generated PDB/rlib files; only linker-diagnosed files under `target/debug` were regenerated. The first complete rerun also exposed the uptime-dependent lock test described above. These failed runs are not counted as passes; final group reports are the authoritative results.

A1 retains its first [failed engineering group](baselines/M2/raw/2026-10-18-a1/rust-initial/checks-rust.json): Clippy found the test probe's duration/midpoint spelling, and MSVC diagnosed one generated SSH-test PDB as corrupt. The source was corrected; only that checked, linker-named generated PDB was removed for regeneration, with a [path/hash record](baselines/M2/raw/2026-10-18-a1/pdb-regeneration.json). The final nine Rust checks pass. `probe-at-reference.rs` preserves the exact pre-lint source used by both fixed reference binaries; no historical sample or threshold was rewritten.

## Gate status and remaining work

| Area | Current status | Still required |
| --- | --- | --- |
| A1 / M2-01 | `accepted` | Exact protocol/limit/failure probes, fixed fair low/high-RTT library references, frozen numeric budgets, policy conformance/payload tests, cwd/editor input contract, audited pinned dependency and prior Explorer feasibility acceptance |
| A2 / M2-02 | `accepted` | Automated lifecycle/OpenSSH evidence plus A-01/A-05/A-06/A-08 manual pass; quantitative mixed-shell measurement remains B6; normal cancellation is now independently drained, while protocol faults/timeouts still quarantine SFTP |
| A3 browse/basic operations | `accepted` | Automated repair/ownership tests plus A-01/A-02/A-07 manual pass; recursive B workflows have separate implementation and pending acceptance |
| A4 single-file safety | `accepted` | Automated hash/fault/commit/conflict tests plus A-03/A-04/A-05 manual pass; pipeline/resume/recovery not included |
| A5 local authority | `accepted` for native picker scope | Automated authority/IPC negatives plus real picker manual pass; production drag/export grants and unsupported platform expansion remain outside acceptance |
| A6 visible Windows + OpenSSH | `accepted` | Maintainer-reported visible connect/browse/picker/upload/download/lock/close pass supplements the real core evidence; no new raw interaction trace is claimed |
| B1–B5 / M2-06–11 | `blocked` (implementation delivered; acceptance pending) | Production pipeline, cwd following, retries/editing, recursive reviewed folders/deletion, native drop/export, partial recovery and shell transactions have implementations. Six real 1 GiB bidirectional resume cases pass. Affected Windows/editor/shell interaction and full quantitative fairness/mixed-load acceptance remain |
| B6–B8 / M2-12–13 | `blocked` | Fixed WinSCP/high-RTT/resource/mixed-terminal acceptance, full affected desktop regression, v0.2 install/portable/upgrade candidate and maintainer sign-off |
| A gate | **`accepted`** | A1 engineering prerequisites plus A2–A6 accepted implementation/manual evidence |
| B / M2 overall | **`blocked`** | Feature implementations delivered; full quantitative evidence, affected desktop acceptance and candidate work remain |

See [M2-A](M2-A.md) for the A-gate record. The three-worker single-file queue does not establish accepted M2-06; the now-closed A prerequisite permits the planned B implementation sequence.

## Known risks and unresolved questions

1. **Cancellation isolation:** normal request cancellation and pipeline drop are independently drained with their admissions and payload reservations retained; siblings can progress and late handles are closed once. A 15-second protocol timeout, malformed state or transport loss can still close SFTP and interrupt sibling file work. Native blocking calls must return before their resources can be reclaimed. Full mixed-load latency/fairness acceptance remains pending; no new SSH authentication is automatic.
2. **Incomplete recovery:** transfer startup reads only its bounded, profile-encrypted two-slot ledger and never scans arbitrary partial names. A discovered upload can resume after a fresh host-matched SFTP session and picker with prefix verification; a user can explicitly remove its deterministic UUID partial after reconnect and checks for regular-file type, size no greater than the source, and size no smaller than the last confirmed offset. On abrupt process exit, that confirmed offset can lag bytes already accepted by the remote pipeline, so the cleanup bound is conservative rather than an exact-size identity proof. SFTP v3 cannot prove the path still names the same remote inode between check and unlink, and a same-size external replacement is not distinguishable; preserve uncertain records and manually inspect them when needed. DPAPI protects metadata under the Windows profile and binds it to its directory/slot, but cannot protect against arbitrary code running as the same user. Missing profile keys, two damaged slots or a copied directory identity prevent discovery while preserving partials. The directory is exclusively held; a second application using that same data directory cannot open another ledger concurrently. New local download records require a fresh exact-target picker and matching saved native file identity; migrated plaintext or missing-original-proof records remain manual-inspection cases. Editor-local marked roots and confidential editor cleanup checkpoints are independent and do not recover transfer queues. Do not delete unfamiliar partials automatically or promise queued-task restoration.
3. **Concurrent external changes:** transfer type/size/mtime comparisons do not detect every equal-attribute content change and are not CAS. Editor baselines additionally compare full SHA-256, but still cannot provide server-side compare-and-swap publication. Local identity checks and pinned ancestors narrow races but do not remove every external check-to-rename gap. Transfer partial cleanup compares lstat with the owned handle’s last attributes. Editor temporary cleanup additionally verifies the full raw-byte hash, preserving equal-attribute content replacements. The final check-to-unlink gap remains because SFTP v3 has no atomic ownership check.
4. **Folder-tree failure boundaries:** recursive enumeration and deletion previews are bounded at 4096 entries and 64 levels; deletion previews additionally bound aggregate path bytes to 256 KiB. Unsupported types are rejected and symlinks are never traversed. Upload creates remote directories before file tasks; download creates local directories before queueing files. A later failure can therefore leave empty directories, a partial transfer tree or retained delete ancestors that require user cleanup. Directory/name/type choices and delete previews are revalidated before mutation. Directory creation/publication and whole-tree deletion are still not atomic transactions; SFTP v3 has no CAS rename/unlink primitive.
5. **Budgets/performance:** three workers, tasks≤1024, grants≤128, final-row pruning≤128 at enqueue, 100 ms progress and 250 ms polling remain enforced. Owned transfer payload admission is instrumented/enforced at 16 MiB; editor snapshots/encoding validation (bounded separately by 8 MiB per file and eight sessions), SSH/wire/OS buffers and process-tree RSS are separate and still require combined peak measurement. Production files use the library's 64-request pipeline with confirmed-prefix progress and convergence. Single-task 1 GiB loopback and 64 MiB high-RTT measurements pass data/end-to-end/absolute gates; complete 1 GiB high-RTT, three-worker, tree and mixed-terminal/resource acceptance remains B6 work. Full readback hashing doubles I/O, and the global publication gate serializes distinct destinations.
6. **Native I/O:** blocking disk calls can exceed five seconds; exit waits up to 30 seconds and keeps the window open on timeout. Source/ancestor handles can block external edits/renames until released. UNC/device/reparse paths and unsupported names require another selection; non-UTF-8 remote names remain unsupported. The application is still Windows x64 scoped.
7. **Durability/metadata:** file fsync plus rename is not proof of a crash-durable directory entry. Ordinary transfer replacement uses the temporary’s mode 0600 and does not automatically retain prior permissions/timestamps. Editor return uploads preserve existing permission bits; timestamps and directory-entry crash durability are not promised.
8. **UI and acceptance:** screenshots remain isolated metadata mocks. The delivered picker/browse/single-file/lock/exit/layout and standalone Explorer checks are now accepted by maintainer report. The exact tested environment/display matrix and raw interaction records were not separately supplied; no broader platform support or quantitative performance is inferred. Actual editor/product drag and new recovery/folder/shell interaction acceptance remain open. Chunk-size and duplicate-dependency warnings remain visible.
9. **Editor recovery storage:** checkpoint confidentiality depends on the Windows user profile. Losing its DPAPI keys, both damaged slots, unsupported future schemas or unsafe native paths can prevent cleanup recovery; the local document is preserved and can be exported/retained. Old pre-checkpoint crashes cannot acquire records retroactively. Retained folders and unknown backups are not automatically purged; disk retention and the 256-root discovery limit require user inspection if many copies accumulate.
10. **Scope identity:** older before snapshots cover selected files/directories. The lifecycle and readiness manifests identify their own saved before files and explicitly new files; inherited source hashes do not manufacture a missing baseline. Historical evidence retains its original hashes and dates. None substitutes for a commit-bound independent review.

The remaining work is the full frozen high-RTT/1 GiB/three-worker/tree/mixed-terminal/resource matrix, actual Windows drop/export and recovered-download interaction, editor and Bash/Zsh lifecycle acceptance, and the v0.2 install/portable/upgrade candidate with maintainer sign-off. Those measurements and packaging are still engineering/acceptance work, not relabeled as completed manual tests. The feature-level gaps enumerated at the previous checkpoint now have code and regression coverage.

## Maintainer checklist for the implemented extension

The delivered Chinese [manual checklist](M2-MANUAL.md) is now accepted as maintainer-reported pass: A-01–A-08, four-file complete hashes and the standalone Explorer feasibility probe. It retains the preparation, operation and cleanup commands for future affected regressions; the unchanged accepted cases are not pending retests.

The [current production-feature review build](baselines/M2/raw/2026-10-20-transfer-protection/review-r1/report.json) contains encrypted transfer recovery, recursive-delete and native/tree UI/commands and is selected by `target/m2-review-current.json`. It remains a version 0.1.0 review package, not a v0.2 candidate; no portable installation, Vault unlock, SFTP desktop workflow or acceptance is inferred from its build success.

The existing **B-edit**, **B-delta**, **B-retry** and transfer-recovery cases retain their pending affected scope. New **B-engineering-01–06** cases at the top of [M2-MANUAL](M2-MANUAL.md) cover native drag/export, reviewed folders, recovered downloads, cancellation co-existence, shell lifecycle and recursive deletion. Earlier A-gate feedback is not extended to these features. Fixed full performance and v0.2 candidate delivery remain outstanding.

Automated permission/read/write/sync/commit/hash failures, stale capability/late-response cases and TCP/subsystem disconnect evidence remain distinct from the maintainer's combined GUI report. This feedback does not invent real full-disk tests, an exhaustive filesystem/network/display matrix or fixed mixed-load latency. Remaining engineering scope is listed above.

`docs/PLAN.md` SHA-256: `2C6A1105A8692C86EBC5A79CA7B5F25C0A99770746D3B9E4F0759BFCB57E126E`.
