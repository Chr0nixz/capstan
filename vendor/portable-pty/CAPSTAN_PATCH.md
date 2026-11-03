# Capstan portable-pty patch

Upstream: `portable-pty` 0.9.0, MIT, wezterm revision
`f8921727a11b9f8b073e8c24821d72fd41283500`, directory `pty`.
The original license and crate metadata are retained. Cargo selects this copy
through the root `[patch.crates-io]` entry.

Changes are confined to `src/win/conpty.rs`, `src/win/psuedocon.rs` and
`src/cmdbuilder.rs`:

- `ConPtySystem::with_conpty_dll` accepts an explicit absolute library path.
  Default construction uses the system API; it never searches for `conpty.dll`.
- Each pseudoconsole owns its function table and loaded module. Creation,
  resizing and destruction therefore always use the same implementation.
- Bundled runtimes use `ConptyCreatePseudoConsole`, `ConptyResizePseudoConsole`
  and `ConptyClosePseudoConsole`; legacy API aliases are also supported.
  See the [Microsoft ConPTY declarations](https://github.com/microsoft/terminal/blob/main/src/inc/conpty-static.h).
- Library or creation failures are returned to Capstan, which retries a new
  session with the system backend. No DLL is copied into the application by
  this patch.
- Fresh terminal panes do not request parent-console cursor inheritance.
  Otherwise initialization requires an asynchronous cursor-position reply and
  can stall before a frontend is attached. This was reproduced by the real
  PowerShell exit test and fixed by removing that flag. See
  [CreatePseudoConsole](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole).
- Environment values are not traced. Process-creation errors retain only the
  operation and OS error, omitting command arguments and working directories.
  These values can contain credentials or other private data.

The unsafe FFI remains confined to the existing vendored Windows adapter.
Capstan crates retain `deny(unsafe_code)`. Supported bundle targets remain
x64 and ARM64; this change does not introduce an x86 release target.

Validation: `cargo test -p capstan-pty --test runtime` checks system creation,
resize, forced close, normal exit/output, fallback from an unloadable PE and
content-free process-creation errors.
An actual audited NuGet package, signatures, version information and ARM64
operation still require the M0 runtime compatibility checklist. The bundle
must include the package's required companion files (including its console
host). No valid bundled runtime is claimed to have been tested by these tests.

On an upstream upgrade, compare these three files against the pinned source,
retain absolute-path selection and per-console ownership, and rerun the above
Windows integration tests. Remove this patch when an upstream API supplies
equivalent behavior.
