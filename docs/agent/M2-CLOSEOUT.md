# M2 closeout

Engineering checks (18/18), scoped independent review, seven nonforeground
cases and engineering package/install/upgrade validation are complete. The
maintainer reports the six affected desktop groups complete.

Low RTT foreground: **PASS**. High RTT foreground: **INCOMPLETE; SSH ACK timeout in formal round 2 at 100% observed focus**.
M1 retains three numerical failures at 100% focus: local throughput
16.087775 MiB/s (minimum 21.688445), local input p95 198.8 ms (maximum 104.6),
and mixed input p95 233.6 ms (maximum 156.9). **M2 remains blocked**.

[Current follow-up](M2-FOLLOWUP.md), [foreground evidence](baselines/M2/raw/2026-10-25-nonforeground-followup/FOREGROUND-STATUS-r5.md)
and [delivery](baselines/M2/raw/2026-10-25-nonforeground-followup/delivery-r5.json) record the exact outcome. Complete mixed
groups include saved-evidence percentile, full-hash and resource/cleanup audits.
The previous high RTT deferral is superseded by the request to continue.
Source, PLAN and frozen limits remain unchanged; candidate and M2 are unsigned.

<details>
<summary>Earlier r3 closeout, retained with its original source/package scope</summary>

# M2 closeout

Engineering fixes, all 18 required checks and production-feature review
packaging have passing records. The post-correction final-source SFTP layout
rerun passes all 36 scenes. All seven final-source nonforeground cases are
complete and audited; all pass unchanged limits. Earlier incomplete
high-RTT attempts remain retained separately. The
maintainer explicitly deferred foreground measurements on
2026-10-24. A1–A6 and the earlier accepted scope remain accepted. Independent
review, affected desktop acceptance and the v0.2 candidate remain unsigned.

## Source and evidence

- Final selected source: 510 inputs, [r3 manifest](baselines/M2/raw/2026-10-23-closeout/source-manifest-r3.json), SHA-256 `9DAB524941913152827FC16668CE90F826E4760949AFEABB1F75AD82E4D9F67F`.
- [Changes](baselines/M2/raw/2026-10-23-closeout/source-changes-r3.json) and exact before/current copies preserve the comparison without inventing a Git revision.
- The [30-file selected-source diff](baselines/M2/raw/2026-10-23-closeout/review-code.diff), [line counts](baselines/M2/raw/2026-10-23-closeout/review-code-stats.json) and [documentation diff](baselines/M2/raw/2026-10-23-closeout/review-documents.diff) make the implementation reviewable; document summaries have separate before snapshots.
- PLAN and frozen performance limits remain unchanged. The Tokio advisory is not an audit exception: [provenance](baselines/M2/raw/2026-10-23-closeout/security-update/provenance.json) records the pinned 1.53.3 upgrade, its MIT license and the failed earlier audits.
- Earlier 510-input single/files/tree low/high-RTT comparisons and six one-GiB resume cases completed successfully under their original identity. The previous status page's collecting/scheduled entries are stale; those results do not automatically accept r3.
- [Final-source quantitative status](baselines/M2/raw/2026-10-23-closeout/QUANTITATIVE-STATUS.md) lists only completed/audited cases, exact profiles and deferred foreground work.
- Six formal file/tree matrices contain 96 samples including warmups and 4,224 full-file hashes; six real 1 GiB TCP-cut/resume cases additionally verify exact appends, new connections and partial cleanup. Each latest attempt is source-bound, and earlier incomplete attempts remain separate.
- The saved-evidence auditor reproduces all seven older complete cases and passes [21 rejection/attempt-selection checks](baselines/M2/raw/2026-10-23-closeout/quantitative-audit-adversarial.json); delivery rechecks evidence hashes to reject stale indexes.

Incomplete high-RTT single-file attempts remain preserved. The original
interrupted attempt left an owned Linux fixture that was stopped using its
recorded owner and the fixture's cleanup scripts. Its
[recovery note](baselines/M2/raw/2026-10-23-closeout/interrupted-fixture-recovery.json)
does not replace the missing execution or final source guard. Automatic
approval initially rejected recursive deletion of the old Windows workspace
with only `blocked by policy`. A subsequent [bounded cleanup](baselines/M2/raw/2026-10-23-closeout/interrupted-workspaces-cleanup.json)
verified both saved owners, removed 52 named files individually and removed
eight directories only after verifying they were empty; no recursion was used.
Both interrupted Windows workspaces, obsolete test accounts and Linux services
have now been removed. Later probe
failures and the WinSCP timeout each retain their own execution/report files.

The first `files-high-r3` attempt was also interrupted after its eight
reference samples, without a completed product sample or final execution
record. Its [separate recovery note](baselines/M2/raw/2026-10-23-closeout/files-high-r3-interruption-recovery.json)
retains the failed standard cleanup after volatile runtime state disappeared.
The old account, namespace and owner-bound processes were absent; its Linux
fixture was then removed after fixed-path and saved-owner checks. The fresh
`files-high-r3-r2` uses a [hidden runner](baselines/M2/raw/2026-10-23-closeout/Launch-FilesHighRetry.ps1)
and new evidence directory. Its completion is determined from the execution
and complete audited samples, never from the runner PID.

The compared total duration includes each client's required full-data
verification. Product samples also retain a second independent post-commit
oracle in `oracleVerificationMs` and `wallMsIncludingIndependentOracle`;
that additional check is outside the comparison after the production engine
has already verified the complete data. Numerical limits and timer boundaries
have not changed.

## Correctness fixes

| Finding | Implementation and regression evidence | Acceptance boundary |
| --- | --- | --- |
| M2-C 7.3.1: cleanup failure loses recovery record | Separate resumable, cleanup-only and manual-inspection records. Keep owned upload attributes and native download identity across restart; require fresh reauthorization; reject externally replaced or unverifiable partials. Current-process final rows no longer hide these records. Successful cleanup refreshes discovery immediately; a nonresumable failed task never regains a misleading Retry button. | Three focused production SFTP regressions plus bilingual controller/component tests; independent review and actual desktop acceptance remain separate. |
| M2-C 7.3.2: concurrent export limit leaves orphan stage | Reserve one of eight slots before stage creation. Release reservations on failure/cancellation; unregistered empty stages attempt bounded ownership-checked cleanup. | Concurrent nine-request regression admits eight exports and leaves zero stages after release. |
| M2-C 7.3.3: expired export remains draggable | Continue observing ready/dragging exports; disable dragging at retained/failed and stop polling terminal states. | Component test covers ready → retained, disabled state, no drag command and stopped polling. |

The download fault adapter forwards the real Windows destination's partial
path, size and identity for the new recovery regression. Older fault variants
keep their original nonresumable interface; the test does not manufacture
native ownership metadata or grant cleanup authority to unknown files.

## Current verification

The [18-check index](baselines/M2/raw/2026-10-23-closeout/engineering-checks.json)
preserves all passing commands, including the post-correction final-source
[SFTP layout rerun](baselines/M2/raw/2026-10-23-closeout/ui-sftp-layout-r3.txt):
fmt, default/runtime/all-feature tests, both Clippy groups,
dependency/license audits, UI typecheck/tests/build/layout/security and native
checks. Rust counts are 341/14/363 (overlapping suites); UI has 238 tests in
38 files, native Windows has 30, and standalone Snap/SFTP vendor suites have
4/3. The new regressions pass within the complete 99-case production SFTP
suite. All 36 SFTP layout scenes pass, including cleanup-only recovery in
English and minimum-width Chinese. External probes left unconfigured by the
default test run remain explicit skips.

The post-Tokio-patch [real OpenSSH core run](baselines/M2/raw/2026-10-23-closeout/openssh-core-r3/report.json)
passes transfer, editor publication/conflict, same-transport reopen and
cleanup. The preceding r2 fixture startup failed and is retained separately;
no product-cause diagnosis is inferred from that failure. All
[17 tooling checks](baselines/M2/raw/2026-10-23-closeout/tooling/report.json),
[manual-fixture tool checks](baselines/M2/raw/2026-10-23-closeout/manual-tools/report.json)
and [16 real local Bash/Zsh probes](baselines/M2/raw/2026-10-23-closeout/shells/shells/report.json)
pass. These isolated probes do not sign off remote desktop installation.

| Gate | State |
| --- | --- |
| Final Rust default/runtime/all-features, Clippy, dependency/license audit | Passed after the Tokio security update |
| Final SFTP layout regression | All 36 scenes pass after the last cache/Retry correction; earlier layout evidence is retained |
| Final production review and opt-in measurement packages | Built and verified against r3; review portable checks pass |
| M1 frozen regression | Fresh r3 attempt failed; foreground only 4/135 and 4/152 samples. Maintainer deferred retest |
| Visible low/high RTT mixed load | Final-binary foreground retests deferred; prior records keep their own identity |
| Final-source transfer/resume performance | Seven complete/audited r3 cases; all pass unchanged gates, full hashes and cleanup; foreground acceptance remains separate |
| Independent review and affected desktop acceptance | Pending |
| v0.2 installer/portable/upgrade candidate and M2 acceptance | Gated |

The failed r3 [M1 execution](baselines/M2/raw/2026-10-23-closeout/visible-m1-r3-execution.json)
preserves its original 21.688445 MiB/s, 104.6 ms local and 156.9 ms mixed
limits. It measured 11.824664 MiB/s isolated throughput, 194/352.4 ms input
p95, but only about 3% foreground coverage. Neither foreground acceptance
nor a causal product regression conclusion follows from that attempt.
All three fixture/network/relay cleanup flags and the final source guard
passed. No foreground rerun starts until the maintainer is available.

## Review delivery

| Artifact | Identity / purpose |
| --- | --- |
| Production review | `target/m2-review-0717a6fe17d6/desktop/Capstan.exe`; SHA-256 `40284F3011FA85768E68F39FEE1EEB114658214B513A9166B38706A4423FD522`; [manifest verification](baselines/M2/raw/2026-10-23-closeout/review-build-r3/identity.json) |
| Portable review | `target/m2-closeout-packages-r3/capstan-0.1.0-m2-review-r3-win-x64.zip`; SHA-256 `CAE581ED07F6EDDB799C90845B6263AE959A19A5A049A51009D10D2C4FD7777E`; [10-file verification](baselines/M2/raw/2026-10-23-closeout/review-portable-check/report.json) |
| NSIS review | `target/m2-closeout-packages-r3/release/bundle/nsis/Capstan_0.1.0_x64-setup.exe`; review-only bundling of the same production executable, without binary patching; no install/upgrade acceptance inferred |
| Opt-in measurement | `target/m2-benchmark-1f6fc6bab424/Capstan.exe`; SHA-256 `64A38D39D7B02DA7ECAF06D216F65CC8191CEFEEC86D1F821796199858CABCF1`; [manifest verification](baselines/M2/raw/2026-10-23-closeout/measurement-build-r3/identity.json) |

The production review rejects `--m0-check` with exit 2. Portable payload,
bundled ConPTY, legal files and executable identity pass. The isolated
[desktop path smoke](baselines/M2/raw/2026-10-23-closeout/review-desktop-smoke/report.json)
passes startup, moving the portable directory and failing closed on a blocked
data path. It does not unlock a Vault or exercise SFTP/terminal interaction.
Both review packages remain version 0.1.0; the planned v0.2 candidate and
v0.1 upgrade acceptance follow the prerequisite B gates. The measurement
artifact is opt-in tooling and is not the maintainer's review executable.
The [package inventory](baselines/M2/raw/2026-10-23-closeout/review-packages.json)
records archive/installer sizes and hashes, including the unchanged production
executable used by NSIS bundling.

## Deferred foreground rerun

When the maintainer is available, run these serially from the repository and
keep the measurement window foreground. Each uses a fresh evidence name;
failed earlier attempts are retained. Do not overlap builds or other test
workloads with sampling.

```powershell
$runner = 'docs/agent/baselines/M2/raw/2026-10-23-closeout/Invoke-Quantitative.ps1'
$measurement = 'D:/Projects/capstan/target/m2-benchmark-1f6fc6bab424/Capstan.exe'
powershell -NoProfile -File $runner -Case m1 -Name visible-m1-r3-r2 -Port 22630 -MeasurementExecutable $measurement
powershell -NoProfile -File $runner -Case visible-low -Name visible-low-r3 -Port 22631 -MeasurementExecutable $measurement
powershell -NoProfile -File $runner -Case visible-high -Name visible-high-r3 -Port 22632 -MeasurementExecutable $measurement
```

The runner rejects selected-source changes. If the source changes before
these measurements, create new source/build identities and fresh case
registrations while retaining the frozen numerical limits. Passing these
measurements still leaves the separate desktop/review/candidate sign-offs.

## Maintainer desktop handoff

Use the package pointed to by `target/m2-review-current.json`. Start its isolated
fixture from the repository with
`powershell -NoProfile -File scripts/Prepare-M2Manual.ps1`, and stop that fixture
after closing its review app with the same script's `-Action Stop`. Record the
actual package manifest, Windows/WebView2, editor and results.

The existing [manual checklist](M2-MANUAL.md) remains authoritative. These six
groups consolidate its outstanding cases without reopening accepted A tests:

1. Explorer file/folder uploads and staged downloads, empty/Unicode names,
   name/type conflicts, cancellation, export-copy cleanup and expired controls
   (`B-engineering-01–03`, `B-folder-01`).
2. Interrupted uploads/downloads, fresh-session/picker retry, changed-prefix
   rejection, restart discovery and explicit cleanup (`B-resume-01–05`,
   `B-retry-01`, `B-transfer-recovery-01–05`, `B-engineering-04`).
3. Actual-editor continuous/rename saves, concurrent remote edits, disconnect,
   lock, export, retained exit and restart recovery (`B-edit-01–08`).
4. Bash/Zsh preview, installation, repeated installation and uninstall while
   preserving added user content; OSC 7 follow/manual browsing and explicit
   aliases (`B-engineering-05`, `B-delta-02–05`).
5. Three transfers with live terminal input, sibling cancellation, tab close,
   layout/preferences, bilingual keyboard operation and safe exit
   (`B-delta-01/06`, affected `B-engineering-05`).
6. Bounded recursive deletion, symlink handling, changed preview, partial
   failure and cancellation (`B-engineering-06`).

Quantitative measurements and fault-injected ownership tests remain engineering
work. Do not manually edit recovery ledgers or use production credentials to
simulate these paths. A maintainer result must identify this new scope; the
earlier A-gate “all passed” report is not reused as its sign-off.

</details>
