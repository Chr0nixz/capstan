# M2 follow-up

Updated UTC: 2026-10-26T08:13:23.164491+00:00. M2 remains **blocked**.
A1–A6 and the accepted A/M0/M1 scope are unchanged. Desktop testing is
maintainer-reported complete. Low RTT passes; high RTT result: INCOMPLETE; SSH ACK timeout in formal round 2 at 100% observed focus.
M1 retains three frozen numerical failures. No commit, push, tag or
publication was performed.

The current selected source is **r5, 510 inputs**, SHA-256 `3EB62CE791B1C085CE652295E2DA8945244A1E356CCA92AD727A06860C07AEA6`.
[Manifest](baselines/M2/raw/2026-10-25-nonforeground-followup/source-manifest-r5.json), [12-file change identity](baselines/M2/raw/2026-10-25-nonforeground-followup/source-changes-r5.json)
and [exact before/current diff](baselines/M2/raw/2026-10-25-nonforeground-followup/review-followup-r5.diff) preserve the
comparison without inventing a Git revision. PLAN and frozen limits are unchanged.

## Completed implementation and review

The authorized [independent read-only review](baselines/M2/raw/2026-10-25-nonforeground-followup/INDEPENDENT-REVIEW.md)
closed the three original M2-C §7 code findings plus two follow-up edges:

- A failed inspection checkpoint cannot restore upload deletion authority
  after restart. Remote deletion requires a cleanup-only record with its
  saved attribute baseline. Resumable uploads retain fresh source/prefix
  verification; they offer no automatic deletion without ownership proof.
- Export construction attempts bounded rollback after the original
  directory identity is captured, covering ACL, marker, sync and directory
  initialization. Unexpected contents and unavailable original identity
  are retained for inspection.
- The export reservation and ready/dragging expiry fixes remain intact.

crypto-bigint is pinned to **0.7.6**, ctutils to **0.4.5** in the lockfile.
The [security provenance](baselines/M2/raw/2026-10-25-nonforeground-followup/security-update/provenance.json) records the
[official advisory](https://rustsec.org/advisories/RUSTSEC-2026-0387.html),
license/archive checks and the permitted tempfile/getrandom reference change.
No audit exception was added. The earlier Tokio 1.53.3 update is retained.

## Engineering and packages

All 18 required checks pass. [Rust checks](baselines/M2/raw/2026-10-25-nonforeground-followup/checks-r5-rust.json),
[UI checks](baselines/M2/raw/2026-10-25-nonforeground-followup/checks-r5-ui.json), [native checks](baselines/M2/raw/2026-10-25-nonforeground-followup/checks-r5-native.json).
The UI suite has 239 tests in 38 files; native Windows has 32 tests.
All 38 SFTP layout scenes pass, including the new upload-recovery English
desktop and Chinese 330×240 cases. Headless Chromium layout screenshots are
engineering evidence; they do not sign off actual WebView2/editor/Explorer use.

The isolated version-only 0.2.0 production engineering installer/portable, ordinary-user 0.1.0 → 0.2.0 upgrade/uninstall and portable path smoke pass. Both original 0.1.0 review packages and the first r3-based
0.2.0 preparation retain their historical identity. The new preparation is
derived from r5 with only four version metadata files changed; it is
**engineering-only, candidate=false**, pending the remaining release gates.

[Engineering build identity](baselines/M2/raw/2026-10-25-nonforeground-followup/r5-r2-build-report.json); [version-only diff](baselines/M2/raw/2026-10-25-nonforeground-followup/version-only-r5.diff).

- [capstan-0.2.0-engineering-win-x64.zip](../../target/m2-version-preflight-20261025-r5/packages/capstan-0.2.0-engineering-win-x64.zip) — engineering-only; candidate=false.

- [Capstan_0.2.0_x64-engineering-setup.exe](../../target/m2-version-preflight-20261025-r5/packages/Capstan_0.2.0_x64-engineering-setup.exe) — engineering-only; candidate=false.

[Actual installation/upgrade/uninstall result](baselines/M2/raw/2026-10-25-nonforeground-followup/install-upgrade-r5-r2/report.json). Seven synthetic files (encrypted Vault/backups/lock, empty recovery slots and panel settings) are checked after install, upgrade and uninstall. This is an existing ordinary Windows user with no prior Capstan data, not a clean OS/new-user or cross-machine sign-off. The first legacy M1 harness rejected the new M2 startup subdirectory at cleanup; its logs and [bounded owned recovery](baselines/M2/raw/2026-10-25-nonforeground-followup/install-attempt-cleanup.json) are retained.

The current main-source 0.1.0 manual-review build is `m2-review-7b5e317ce4f9`; [verified manifest](../../target/m2-review-7b5e317ce4f9/manifest.json). The pointer `target/m2-review-current.json` now selects it. This separate review executable matches the r5 source without the isolated version metadata override.

The refreshed opt-in measurement build is `m2-benchmark-0538b0e0b5cd`; [build record](baselines/M2/raw/2026-10-25-nonforeground-followup/measurement-build-r5/report.json). Its capability preflight is not a foreground measurement.

## Final-source nonforeground measurements

7/7 final r5 nonforeground cases complete and audited; 7/7 pass unchanged limits. The preceding r3 source's
[seven passing cases](baselines/M2/raw/2026-10-23-closeout/QUANTITATIVE-STATUS.md)
remain valid only for that source; they are not relabeled as r5 results.
Sampling is serial, after builds/package tests, with fresh names and fixtures.

The first r5 resume attempt stopped after two retained upload cuts and never
produced a final execution record. Its failed worker and cleanup logs remain
retained. A [separate recovery](baselines/M2/raw/2026-10-25-nonforeground-followup/resume-interruption-postcleanup.json)
verifies the old fixture/account/processes/namespace/route and Windows
workspace are absent; it does not reconstruct the missing execution record.
The continuation preserves the successful tree case and uses a fresh resume
attempt name before the remaining five cases.

The first files-low and tree-high attempts were invalidated by the concurrent
compiler guard; both verified source identity and fixture cleanup. Their
partial samples and failed records are retained. Any retry uses a fresh name
and repeats the entire matrix after a compiler-free preflight; no samples
from different attempts are combined and no numerical limits are changed.

| Case | Current state | Evidence |
| --- | --- | --- |
| single-low | PASS | [single-low-r5](baselines/M2/raw/2026-10-25-nonforeground-followup/single-low-r5-recomputation.json) |
| files-low | PASS | [files-low-r5-r2](baselines/M2/raw/2026-10-25-nonforeground-followup/files-low-r5-r2-recomputation.json) |
| tree-low | PASS | [tree-low-r5](baselines/M2/raw/2026-10-25-nonforeground-followup/tree-low-r5-recomputation.json) |
| resume-low | PASS | [resume-low-r5-r2](baselines/M2/raw/2026-10-25-nonforeground-followup/resume-low-r5-r2-recomputation.json) |
| single-high | PASS | [single-high-r5](baselines/M2/raw/2026-10-25-nonforeground-followup/single-high-r5-recomputation.json) |
| tree-high | PASS | [tree-high-r5-r2](baselines/M2/raw/2026-10-25-nonforeground-followup/tree-high-r5-r2-recomputation.json) |
| files-high | PASS | [files-high-r5](baselines/M2/raw/2026-10-25-nonforeground-followup/files-high-r5-recomputation.json) |

## Foreground and desktop results

The maintainer [reported the six affected desktop groups complete](baselines/M2/raw/2026-10-25-nonforeground-followup/desktop-maintainer-r5.json).
The current high RTT request supersedes the earlier deferral. Individual
desktop case/platform matrices were not supplied.

| Group | Current outcome |
| --- | --- |
| M1 | Numerical FAIL with 100% focus; throughput/local-input/mixed-input gates remain open |
| Low RTT mixed | PASS; four rounds at 100% focus, full hashes and saved-evidence audit |
| High RTT mixed | INCOMPLETE; SSH ACK timeout in formal round 2 at 100% observed focus; source and fixture/network/relay cleanup verified |

See [exact foreground metrics and source-bound evidence](baselines/M2/raw/2026-10-25-nonforeground-followup/FOREGROUND-STATUS-r5.md).
M1 measured 16.087775 MiB/s against a 21.688445 minimum and input p95
198.8/233.6 ms against maxima 104.6/156.9. This numerical failure remains open.

## Remaining acceptance

- Resolve three M1 frozen numerical failures.
- Investigate high RTT SSH ACK timeout and complete a valid visible mixed measurement.
- Candidate promotion/sign-off after prerequisite gates.

[Current delivery](baselines/M2/raw/2026-10-25-nonforeground-followup/delivery-r5.json) preserves the preceding delivery
identity and the actual high RTT result. Candidate and M2 remain unsigned.
