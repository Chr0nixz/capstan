# Capstan bounded-file adapter patch

The M2 closeout pins Tokio to 1.53.3 with the rest of the workspace for
RUSTSEC-2026-0386. This patch-level dependency update changes no vendor SFTP
source or protocol/resource policy; Tokio remains MIT licensed.

Base: crates.io `russh-sftp` **3.0.1**, Apache-2.0. The downloaded archive SHA-256 is `9bb17f72aba4314656b359ffba72c68dcbd70ed9e97497898b75498fe7198042`; `.cargo_vcs_info.json`, `Cargo.upstream.toml`, `Cargo.toml.orig`, LICENSE and the upstream source inventory are retained. Runtime dependencies are pinned to the versions already in Capstan's lockfile; upstream examples/benchmarks and their unrelated dev dependencies are excluded. The optional async-trait feature uses the repository's pinned 0.1.89 macro, validated separately. No second SSH implementation is introduced.

The existing upstream `File` read/write windows, `RawSftpSession`, request matching and SFTP serialization remain the implementation. The patch adds:

- Construction of a `File` from an already initialized raw session and owned handle, with caller-supplied negotiated request size/window configuration. Capstan's facade retains its authenticated lease, handle ownership, 64-request file admission, 208-slot global admission and cancellation/error handling; no second handshake/OPEN occurs.
- Contiguous successful write-ACK offsets. A failed ACK stops prefix advancement; sent/queued bytes are never a resume boundary or completed progress.
- Explicit convergence of issued read/write requests before CLOSE, including pending reads beyond EOF and known STATUS failures. Status codes remain typed through `io::Error`; Capstan strips server error text at its existing wire boundary.
- Short reads retry only the missing range at the front of the existing window. Later requests remain owned/countable; the patch does not discard and reissue the entire window. Oversize replies fail closed.
- A checked 64-bit internal request-ID allocator that refuses numeric IDs after u32::MAX. Every allocation site propagates failure, including requests rejected before sending; IDs never wrap back to a reused value.

Application payload admission reserves a complete maximum DATA window before pipeline use. The framing adapter rejects DATA bodies over 32 KiB before library parsing. Three file windows leave 16 control slots; core yields between blocks. A normally closed file converges requests before its payload/request reservations are returned. Capstan now drains normal canceled requests/files independently and holds the reservations until convergence/CLOSE. EOF, malformed state or an undrainable protocol timeout still quarantines shared SFTP; adjacent SSH shells remain independent. This lifecycle policy is in the Capstan facade, not a second vendor request dispatcher.

Regression evidence includes four deterministic facade pipeline peers (full window/ACK progress, reverse replies and short-read gaps, failed ACK holes, three windows plus control/sibling progress), the production in-process pool/service/native-I/O workflows, wire DATA bounds, and a raw allocator boundary unit test. See `docs/agent/M2-B.md` and its linked pipeline evidence for exact executed checks, real-server runs and performance limits. This patch is not a claim that folder/drag/resume/editor workflows are implemented.

## Transport pacing hook

`client::Pacer` lets the caller limit pipelined file data that shares one SSH transport with interactive channels. A `File` built `with_pacer` asks before each READ or WRITE beyond its first outstanding request. A refused write waits for the file's oldest acknowledgement; a refused read is refilled by the file's next reply. No extra waker is needed, and a file never stalls on the pacer. A short read's gap request is admitted regardless, because it continues an admitted request. Without a pacer, `File` behaves as before.

Each admission is a ticket held in the request's reply slot. The reply dispatcher returns it with the measured round trip, even while the file is not being polled. Dropping the request, a timeout or a failed send returns it without a round trip. A failed send now also removes its reply slot, which upstream left in the request map. The delay-based controller is Capstan code in `crates/capstan-ssh/src/sftp/pacing.rs`.

Regression evidence: the vendor test `every_admission_is_returned_once_with_a_round_trip_only_for_a_reply` covers reply, drop and unsent paths. The capstan-ssh paced pipeline tests cover write waiting, read limits and gap admission. The capstan-core `slow_link` tests run transfers over an emulated 1 MiB/s, 80 ms link.

## chrono removed

RUSTSEC-2026-0543 marks `chrono` unmaintained, with no fixed release. Upstream used it in two places. One converts `SystemTime` to protocol seconds; the other formats the server-side `longname` as `%b %d %Y %H:%M` UTC. Both are now plain `std` code in `utils.rs`. The conversion keeps chrono's rounding down before the epoch and its 32-bit truncation. Before removal, the replacements matched chrono on about 41,000 timestamps sampled across the full 32-bit range and on sub-second values either side of the epoch. Fixed-value tests remain in `utils.rs`. Capstan's client path uses neither helper.
