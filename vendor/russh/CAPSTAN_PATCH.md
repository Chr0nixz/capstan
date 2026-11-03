# Capstan russh patch

Upstream: `russh` 0.61.2, Apache-2.0, crates.io source from upstream commit
`ff74d7332b717fe6caf56f63aa4decdcdfab8645`. The source and normalized Cargo
manifest are retained; `Cargo.toml` selects this copy through the root
`[patch.crates-io]` entry. The Apache-2.0 license text is retained in
`LICENSE-APACHE`.

Capstan changes the client channel receive path:

- Client channel data frames are delivered with non-blocking sends. If a
  consumer is behind, adjacent stdout or same-code extended-data frames merge
  in a per-channel queue; control message ordering is preserved. Pending
  control items have a fixed count limit, and the bytes held for data are
  limited by the SSH receive window because window credit is no longer
  returned at packet-dispatch time.
- A channel's receive window is replenished only as its reader consumes bytes.
  This lets a slow shell reach its receive-window limit while the shared SSH
  packet loop continues to dispatch sibling channels and protocol messages.
- Close/EOF and status messages use the same non-blocking queue, so they do not
  wait behind a full channel queue in the connection's packet loop.
- Agent identity responses, public-key material, and signing payloads are not
  written to debug logs; the upstream debug statements included these values.

The buffer bound covers russh's configured channel message queue, pending
coalesced bytes up to the advertised SSH receive window, and one in-flight SSH
packet. It does not include Capstan's downstream terminal queue, the OS socket
buffers, or renderer memory; those have separate budgets. Server-side channel
handling is unchanged.

Validation is provided by Capstan's process-local russh server regression
`a_saturated_shell_does_not_block_another_shell_on_the_same_transport`, which
has one shell emit 8 MiB without a reader while a second shell completes a
request/response on the same transport. Unit tests also cover adjacent-frame
coalescing, control ordering, and pending-control limits. These tests do not
replace real OpenSSH compatibility or Windows/WebView2 acceptance.

## Full duplex client output progress

The client now polls socket output alongside packet reads, channel-consumption
notifications and timeouts. It stops admitting application batches while output
is pending, rather than awaiting a complete socket write before reading again.
This avoids reciprocal socket backpressure stalling SFTP and sibling shells.
PacketWriter retains its written cursor across interrupted flush futures, so
already written ciphertext is never replayed. Each poll writes at most 64 KiB
before yielding, and application batching targets 64 KiB plus one message.

At most 64 KiB of new control output may accumulate behind an admitted batch.
The accounting includes channel data deferred by remote windows; a window
adjust moves that existing admission into packets without charging its payload
again. Consumed-byte window credit continues during socket backpressure, and
rekey preserves the ordering of deferred data and encrypted packets. Transfer
worker, request, remote-handle and payload limits are unchanged.

The in-process `duplex_progress` integration tests use a 1 KiB bidirectional
transport, exchange 8 MiB each way, and check a sibling echo both with and
without repeated rekey. The original client loop times out in this fixture.
PacketWriter tests cover interrupted partial writes, interrupted socket flush
and packet appends without duplicated prefixes. A window-adjust regression
checks the deferred-data admission accounting.

## RSA backend replacement

The optional `rsa` feature retains SSH RSA support but no longer depends on the
RustCrypto `rsa` package. PKCS#1/PKCS#8 conversion delegates to the patched
ssh-key library's AWS-LC key adapter. See `vendor/ssh-key/CAPSTAN_PATCH.md` for
key validation, SHA-2 signing, supported sizes and verification boundaries.
