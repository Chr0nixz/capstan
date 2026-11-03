# Capstan native Windows boundary

## M2 product export and recovered destination (2026-10-19)

`export` owns protected user/SYSTEM staging with bounded basename-only D/F/P
manifests, exclusive markers, reader pins and explicit nonrecursive cleanup.
Startup discovery excludes live owners and unknown/unsafe content. OLE return,
Escape and expiry preserve files. Cleanup opens every known file exclusively
before deletion; the final root removal revalidates its original volume/file
identity and deletes through that same exclusive directory handle, closing the
path replacement gap after traversal pins are released.

`transfer` adds an unmodified selected-directory capability for reviewed plans.
A recovered destination must be freshly picker-authorized for the original
final path; it anchors the sibling partial, opens it exclusively, verifies its
saved identity, link count and size bounds, and preserves it on abandoned
selection. No arbitrary path or file body enters the WebView. Native validation
now runs 27 tests; production drop receipts, tree decisions and resumed byte
prefixes are tested separately in core. Actual Explorer copy/cancel/reader
lifetime and desktop recovery remain maintainer acceptance.

## M2 editor staging (2026-10-19)

`editor` adds random UUID session directories below the Capstan editor temp
root. A protected DACL grants the current Windows user and SYSTEM access before
document bytes are written; child files inherit that ACL. Ancestor pins reject
reparse points and prevent directory substitution. Reads pin the actual regular
file without allowing writes/deletes during the bounded snapshot, while
releasing it between observations so rename-save editors remain usable.

The selected absolute `.exe` is started with one absolute document argv entry;
there is no shell command string. Standard input/output/error are disconnected
from the parent so an external editor cannot write document output into app logs. The child process exiting is not proof that a
shared-process editor has finished. The capability does not kill that editor.

An exclusive `.capstan-owner` handle excludes live application instances.
Recovery reads only a bounded, versioned `.capstan-editor` marker containing the
document basename, without persisting a host or remote path. Discovery inspects
at most 256 roots and admits at most eight sessions in core. Invalid markers,
reparse paths and unknown children are retained. Cleanup targets only the known
files through opened handles and a nonrecursive directory remove. A local export
uses the ordinary anchored transfer destination with exclusive publication; a
document replaced since the export snapshot cannot be deleted by that cleanup.

Validation is part of `Check-M2.ps1 -Group native`: fmt, all-target Clippy and
22 native tests. The editor tests cover protected ACL, in-place/rename saves,
live-instance exclusion, forged markers, failed cleanup and exclusive local
copies. Actual VS Code/Notepad++ launch/save/share behavior remains a separate
maintainer test. Filesystem calls may exceed the cancellation target, and
unrecognized editor backup files intentionally prevent automatic cleanup.

## M2 confidential editor cleanup recovery (2026-10-19)

`editor/recovery` owns two exclusive, single-link, non-reparse checkpoint files
inside the protected staging directory. Current-user DPAPI encryption uses
`CRYPTPROTECT_UI_FORBIDDEN`, no machine-wide flag, no prompt or description,
and entropy bound to the session UUID and document basename. Native allocations
and decrypted buffers are zeroized. Core frames additionally validate a sequence
hash, schema, session and bounded cleanup records. A checkpoint is at most 64 KiB
clear / 96 KiB sealed per slot. Writes sync the inactive slot; older delayed
writers cannot replace a newer sequence. Core mirrors each new path to both
slots before remote OPEN, so a damaged/torn slot can retain the other intent.

Recovery slots are held for the session lifetime and reject hard links before
reading/writing. Document snapshots and cleanup now also reject hard links;
external linked files are not read for upload or deleted. Explicit retain closes
the core workflow and leaves all files for future discovery. Unknown children
still prevent ordinary cleanup. Losing the Windows profile keys or both valid
slots prevents metadata recovery, preserving the document for export/retention.
File sync does not guarantee a crash-durable parent directory.

The implementation follows [CryptProtectData](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata)
and [CryptUnprotectData](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptunprotectdata).
The latter recommends application-level integrity validation; core checks the
frame hash before deserialization. No plaintext credential material or document body is serialized
into checkpoint metadata. This adds only `Win32_Security_Cryptography` to the
existing pinned `windows-sys` version. Native tests cover encrypted round trips,
torn slots, swapped-session ciphertext, bounds, stale writers and hard links;
a core test separately verifies abrupt child-process exit and discovery.

## M2 standalone manual probe (2026-10-18)

`examples/m2_native_drag.rs` supplies a small bilingual Win32 window for actual
file, directory and multi-item Explorer drop tests plus Escape cancellation.
It is a separate executable, not an application command or shipping drag
workflow. The existing Shell data-object adapter is reused. Setup diagnostics
now retain only the failed HRESULT, without paths or file data. The additional
LibraryLoader feature supports this example's window class.

`Test-M2NativeDrag.ps1 -SelfTest` constructs all three data objects and checks
that source bytes remain; it cannot accept an Explorer drag. Interactive mode
records native copy/cancel outcomes, checks Unicode/nested/empty-directory
destinations and full SHA-256 after copying, and retains all staging. It does
not delete staging merely because the OLE call returned. Actual SFTP export
authorization, download preparation, quotas, asynchronous-consumer lifetime
and recovery are now implemented by the product extension above; this standalone probe does not accept those workflows.

This repository-owned crate contains the narrow Windows FFI adapters used by
the application. It is excluded from workspace membership; validate it with
`cargo fmt --manifest-path vendor/capstan-native-windows/Cargo.toml -- --check`
and `cargo test --manifest-path vendor/capstan-native-windows/Cargo.toml`.

## M2 native publication path termination (2026-10-20)

The Win32 `FILE_RENAME_INFO` allocation now reserves an explicit wide NUL after
the DOS destination path, independently of usize alignment padding.
`FileNameLength` still counts only the path bytes. The real 128-file tree
download exposed intermittent `ERROR_INVALID_NAME` at native publication;
the in-process three-file tree reproduced it before this correction and passed
30 consecutive repetitions afterward. Native tests cover eight consecutive
filename lengths, hidden empty and nonempty files, and a Unicode/space parent.
No retry, path-validation or overwrite check is bypassed. This follows the
[FileName string contract](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info)
and does not rely on unallocated or incidental zero-filled padding.

## M2 picker-selected file transfer (2026-10-17)

`transfer` adds narrowly scoped Windows open/save pickers and regular-file
capabilities. The trusted caller supplies bilingual dialog strings. Paths stay
in Rust; the WebView receives a one-use, owner-bound core token and a basename.
Server text is never used as a picker filter or default filesystem path.

Source and destination capabilities pin every ancestor from the drive root
with handles that deny directory deletion/rename. Final opens use
`FILE_FLAG_OPEN_REPARSE_POINT`; reparse points, directories as files, UNC/device
paths, invalid or reserved Windows names, ADS, traversal, excessive path length
and depth are rejected. Sources deny writes/deletion for their lifetime.
Downloads use exclusive same-directory temporary handles; sync, readback,
rename and deletion use the held handles. Before `FILE_RENAME_INFO` publication,
the destination's volume/file identity, size, write time and attributes must
still match the approved version. Existing files are never deleted first.

Eight native tests pass (including the two staged-drag probes): real temporary
file publication/cleanup, a target appearing or changing, equal size/time with
a different file identity, pinned ancestors/source sharing, Windows aliases
and a runtime-created junction. Evidence: `docs/agent/baselines/M2/raw/2026-10-17-transfers/native-final/`.
These tests do not simulate a user completing a picker or Explorer workflow.
Held ancestor/source handles can block external rename or edits until the
grant/task is released. Blocking disk I/O does not establish the proposed
five-second cancellation SLA. The checks do not eliminate all external
filesystem TOCTOU or guarantee parent-directory durability after rename.

## M2 staged drag probe (2026-10-17)

`staged_drag` creates a Shell IDataObject from bounded, already-staged files and
directories and offers a copy-only SHDoDragDrop entry point. OLE initialization,
PIDLs and the data object have apartment-bound RAII lifetimes. Tests use the
actual Shell implementation to extract CF_HDROP and compare exact Unicode
paths, including both a file and directory; they do not simulate user acceptance
or claim an Explorer copy occurred.

This is a platform prototype without an application command. Core must still
authorize export tokens, retain the owned staging tree while consumers may use
it, prevent concurrent mutation/reparse replacement, bound disk usage and handle
recovery/cleanup. Preflight path checks here do not alone eliminate filesystem
TOCTOU races. Returning from OLE does not authorize immediate staging deletion.
