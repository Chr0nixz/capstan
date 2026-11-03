# Vault format and recovery

This document describes the format implemented by `capstan-vault` in M1. The format is versioned and bounded; it is not a promise of compatibility with a published release. No M1 release has been published.

## Encrypted envelope

Vault files use the `CAPSTAN1` magic. Independent `.cbak` exports use `CBSTBAK1`. Both use a fixed 160-byte header followed by authenticated ciphertext. Integers are little-endian.

| Offset | Length | Field |
| --- | ---: | --- |
| 0 | 8 | Envelope kind magic: `CAPSTAN1` or `CBSTBAK1` |
| 8 | 2 | Envelope version; currently `1` |
| 10 | 2 | Data schema; currently `1`; authenticated schema `0` is migrated in memory |
| 12 | 16 | Random file identifier |
| 28 | 4 | KDF identifier/version/reserved bytes: `02 13 00 00` (Argon2id, version 19) |
| 32 | 4 | Argon2 memory in KiB |
| 36 | 4 | Argon2 iterations |
| 40 | 4 | Argon2 parallelism |
| 44 | 16 | Random Argon2 salt |
| 60 | 24 | Random XChaCha20-Poly1305 nonce for the wrapped data-encryption key |
| 84 | 24 | Random XChaCha20-Poly1305 nonce for the JSON payload |
| 108 | 4 | Ciphertext length, including the 16-byte authentication tag |
| 112 | 48 | 32-byte data-encryption key wrapped with a 16-byte authentication tag |
| 160 | variable | XChaCha20-Poly1305 ciphertext for the serialized `VaultData` |

The master password derives a 32-byte key-encryption key (KEK) using Argon2id v19. The default parameters are 65,536 KiB, 3 iterations and parallelism 1. The accepted bounds are 19,456–262,144 KiB, 2–6 iterations, parallelism 1–4, and `memory_kib × iterations <= 1,048,576`. New master passwords must contain at least 12 characters and at most 1,024 UTF-8 bytes. Files larger than 16 MiB, malformed lengths, unknown envelope versions and out-of-policy KDF parameters are rejected before key derivation or large allocation.

M3 adds snippet groups, snippets and tunnel configurations to the encrypted model without changing the schema number. Missing lists decode as empty, so earlier payloads still open. Empty lists are not written, so a Vault without M3 data stays readable by earlier builds. Earlier builds reject unknown fields, so once a snippet or tunnel is saved they can no longer open that Vault or its later backups. To keep a way back, export a `.cbak` before saving the first snippet or tunnel.

The independently generated 32-byte DEK encrypts the serialized model with XChaCha20-Poly1305. Its AAD is header bytes 0–27. The wrapped DEK uses the KEK and its separate nonce; its AAD is header bytes 0–111 followed by SHA-256 of the payload ciphertext. This authenticates the KDF metadata and binds the wrapped DEK to the payload. Changing the master password derives a new KEK and rewraps the same DEK; the payload ciphertext and its nonce stay unchanged.

## Durable writes and recovery files

Transactions acquire a sidecar file lock, compare the on-disk digest and model revision, serialize a new validated model, and write a unique same-directory `*.pending` file. The temporary file is flushed and synced before an atomic replacement. On Windows, replacement uses the native atomic-replace adapter. Unix also syncs the containing directory; Windows directory syncing is not implemented. A failed write does not intentionally replace the main Vault.

Before replacing the main file, the store archives the previous encrypted envelope as `<vault-name>.backup-<sequence>-<mac>.cav`. The filename MAC is keyed by the DEK and covers a domain label, sequence number and encrypted-file digest. At most three authenticated recovery backups are retained; invalid or unauthenticated files do not evict a valid backup. A crash-time temporary file ends in `.pending`.

Recovery candidates are enumerated only after successful decryption and model validation. Restoring a candidate checks its revision and, for a backup, its DEK-authenticated filename before atomically replacing the main file. A backup made before changing the master password remains wrapped with the earlier password; use that earlier password to authenticate that backup. A pending file is never promoted automatically.

The recovery scan examines at most 128 directory entries and performs password authentication for no more than eight matching candidate files per request. Exceeding either bound returns an error; the user can move older files elsewhere and retry. This keeps recovery work bounded even when the data directory contains unexpected files.

The locked-vault UI can explicitly search for recovery files after unlocking fails. The user enters a candidate password in a native dialog; only authenticated candidates are returned, represented by a short-lived opaque token, kind and revision. File names and paths stay in Rust. Restoring requires a separate confirmation and re-entry of the candidate password. Rust authenticates the same file again, checks that it belongs to this Vault, then atomically replaces the main file and leaves the Vault unlocked. Failed authentication or an expired/stale token does not replace the main file. Backups from before a master-password change may require the earlier password. Candidates are never promoted automatically.

## `.cbak` export and import

An exported `.cbak` is a separately sealed envelope with `CBSTBAK1` magic and an independent password. It does not copy the main file or share its DEK. Import first decrypts and validates the complete backup, then builds a conflict preview. Merge remaps colliding IDs and references. It reuses a snippet group with the same name and skips identical snippets and identical forwards on the same host, so importing a backup twice adds no copies of them. Overwrite is an explicit replacement and preserves the current valid Vault as a recovery backup. Host-key conflicts block commit rather than silently widening trust. A wrong password or canceled preview leaves the current Vault unchanged.

## Errors and limits

Stable user-facing error codes include `vault_format`, `vault_version`, `vault_kdf_policy`, `vault_authentication`, `vault_data`, `vault_conflict`, `vault_recovery_expired`, `vault_recovery_limit` and `vault_io`. Error messages do not include passwords, decrypted data or file contents. The implementation has unit tests for header bounds, tampering, wrong passwords, schema-zero migration, nonce freshness, rewrapping, backup rotation and injected failures before/after atomic replacement. KDF defaults have not been calibrated on the maintainer's target device, and the 16 MiB limit is a parser bound rather than a tested maximum Vault capacity.
