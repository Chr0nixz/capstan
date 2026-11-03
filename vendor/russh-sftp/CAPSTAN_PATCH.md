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
