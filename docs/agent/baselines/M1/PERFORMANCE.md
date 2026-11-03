# Fixed visible M0 reference and M1 comparison — 2026-10-17

The accepted run is [r4](raw/2026-10-17-visible-performance-r4/summary.json).
The [premeasurement policy](performance-policy.json) was written before the
first measurement. Each mixed run started only after a completed local reference
and frozen numeric limits. R4 reuses R3's exact numeric limits; no failed limit
was loosened. The frozen-policy hash chain and original failures are retained.

This re-establishes the missing M0-compatible local reference on the current
device. Both reference and M1 use the same release measurement executable,
ConPTY package, 1280×800 CSS viewport, DPR 1.5 and WebView2 156.0.4314.23.
It compares isolated/local and mixed workloads, not a historical before/after
comparison of two source revisions. The production candidate excludes the
measurement IPC; its own identities and package verification are separate.

| Measurement | Result |
| --- | --- |
| M0 local reference | 62 sessions, 186 probes, three rounds of 20; binary integrity/ACK/cleanup pass |
| M1 | 63 sessions, 192 probes; three mixed rounds of 4 local + 8 password SSH + 8 Ed25519 SSH tabs |
| Mixed bulk | Each round generates 100 MiB locally and 100 MiB over SSH; source/receiver counts, sequence, checksum and ACK agree |
| Isolated local output | 28.279 MiB/s; frozen minimum 21.688 MiB/s |
| Mixed local output | 12.124 / 12.091 / 11.787 MiB/s |
| Mixed SSH output | 13.068 / 13.384 / 12.813 MiB/s |
| M1 input response | Median 77.9 ms; p95 129.3 ms, frozen limit 156.9 ms |
| Local input in mixed rounds | p95 103.4 ms, frozen limit 104.6 ms |
| Normal sibling during slow ACK | p95 34.3 ms; same SSH connection, frozen limit 1000 ms |
| Peak process tree | 925,589,504 working-set bytes; 5,417 handles; includes WebView2 and owned children |
| Visibility | All 134 reference and 136 mixed IPC observation samples visible/focused |
| Ownership | One visible renderer maximum; renderers/sessions return to zero; SSH pool drains from two to zero |

CPU is Intel Core i7-14700HX (28 logical processors), Windows build 28020,
WSL kernel 6.18.33.2; server OpenSSH_9.6p1 Ubuntu-3ubuntu13.20. Actual negotiated
algorithms are recorded per source: curve25519-sha256 and
chacha20-poly1305@openssh.com. Negotiated MAC names are also retained; AEAD
integrity belongs to the cipher. Network conditions are loopback Windows→WSL;
`network.json` contains ten TCP-connect latency samples (explicitly not ICMP
RTT). Input probes separately include the entire request/response and xterm
parsing path. No WAN, physical-keyboard, IME or GPU-presentation latency claim.

The application queue limit is 1 MiB plus one 64 KiB frame; russh separately
uses a 2 MiB initial channel window, 32 KiB maximum SSH packet, 100 configured
channel message slots and a 32-item downstream shell queue. Pending data is
bounded by receive-window credit as described in the vendored russh patch;
these are configuration bounds, not a total RSS promise. TCP_NODELAY is enabled
for interactive traffic. The local input limit has little headroom on this
device and must remain in subsequent regression checks.

Earlier evidence remains immutable: the first attempt failed while preparing
the pinned SSH fixture; r2 exceeded the server's default ten sessions on one
identity; r3 passed integrity/topology/cleanup but failed the input p95 limits.
File-comment normalization, balanced 8+8 shell allocation and TCP_NODELAY fixed
those causes. R4 passed all 28 comparison checks without loosening thresholds.
The fixture account, listener and credential directory were cleaned after each
attempt. No terminal transcript or secret is included.

Reproduction uses the opt-in `m0-benchmarks,custom-protocol` release build and
the `capstan-m0-fixture` release binary. Copy both into a new directory beneath
`target`, write the exact `CAPSTAN_PORTABLE_V1` sibling marker, and copy the
verified `runtime/x86_64` tree. Do not use an existing user Vault. Then run:

```powershell
powershell -NoProfile -File scripts/Measure-M1.ps1 -Executable <isolated-directory>/Capstan.exe -OutputDirectory <new-evidence-directory> -PreviousFrozenPolicy docs/agent/baselines/M1/raw/2026-10-17-visible-performance-r4/frozen-limits.json
```

The wrapper starts a disposable pinned WSL sshd, captures the reference, freezes
limits before the mixed run, samples owned processes and verifies the results.
Production binaries reject the measurement preflight. The fixture requires
Ubuntu/WSL and a free loopback port; it does not alter a normal sshd service.
