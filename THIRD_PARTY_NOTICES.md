# Third-party notices

## russh-sftp 3.0.1 (M2 development probes only)

- License: Apache-2.0, as declared by the pinned crates.io package.
- Source: https://github.com/AspectUnk/russh-sftp ; published package
  https://crates.io/crates/russh-sftp/3.0.1 .
- Used only by in-process protocol characterization tests. It is not an
  application runtime dependency or an accepted M2 protocol implementation.
- Non-UTF-8 filename and opaque-handle limitations are reproduced in
  `crates/capstan-ssh/tests/sftp_protocol.rs`; see `docs/agent/M2-SFTP-COMPAT.md`.

## portable-pty 0.9.0

- Copyright (c) 2018 Wez Furlong.
- License: MIT; full text retained in `vendor/portable-pty/LICENSE.md`.
- Source: https://github.com/wezterm/wezterm/tree/f8921727a11b9f8b073e8c24821d72fd41283500/pty
- Capstan modifications: explicit ConPTY runtime selection, module ownership,
  and removal of environment values and process arguments from diagnostics;
  see `vendor/portable-pty/CAPSTAN_PATCH.md`.

## Zustand 5.0.8

- License: MIT, verified from the installed package metadata and LICENSE.
- Source: https://github.com/pmndrs/zustand
- Used for serializable workspace/tab state; terminal bytes and xterm instances
  are not stored in Zustand.

## ConPTY runtime binaries

- Microsoft.Windows.Console.ConPTY **1.25.260930003**, from the official NuGet
  package: https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY/1.25.260930003
- License: MIT, copyright Microsoft Corporation. Full text is retained in
  `resources/ConPTY-LICENSE.txt` and distributed as `runtime/x86_64/LICENSE.txt`.
  Upstream license: https://github.com/microsoft/terminal/blob/main/LICENSE
- The pinned package, payload hashes and architectures are recorded in
  `resources/conpty.lock.json`. `scripts/Prepare-ConPty.ps1` verifies the package
  hash and each binary's Microsoft Authenticode signature, PE architecture and
  file version before packaging. Binary files are generated under `target`,
  not checked into source control.
- Windows x64 bundles contain `runtime/x86_64/conpty.dll`,
  `runtime/x86_64/x64/OpenConsole.exe` and
  `runtime/x86_64/arm64/OpenConsole.exe`, retaining the package's native-host
  companion layout. The ARM64 companion is required for native-host selection;
  its inclusion does not imply an ARM64 Capstan build has been tested.
- The application prefers this sibling runtime and falls back to the system
  ConPTY if loading fails. `manifest.json` records the verified package version
  and provenance. Release-time dependency license auditing remains required.

## tauri-plugin-snap-layout 1.0.9 (restricted Capstan fork)

- License: MIT; full text and original copyright notices retained in
  `vendor/tauri-plugin-snap-layout/LICENSE`.
- Source: https://github.com/Hyph-M/tauri-plugin-snap-layout (published 1.0.9).
- Used only as a native Tauri window adapter. Domain crates do not depend on it.
- Changes and verification boundary: `vendor/tauri-plugin-snap-layout/CAPSTAN_PATCH.md`.
- Direct dependency versions are pinned in the adapter manifest. Tauri,
  tauri-plugin, windows-sys, serde and thiserror use MIT/Apache-2.0 terms; tokio
  uses MIT. This addition does not replace release-time `cargo deny` auditing.

## russh 0.61.2 (Capstan patch)

- License: Apache-2.0; full text retained in `vendor/russh/LICENSE-APACHE`.
- Source: crates.io release 0.61.2, upstream revision
  `ff74d7332b717fe6caf56f63aa4decdcdfab8645`.
- Used for SSH client/server transport with the `ring` and `rsa` features.
- Capstan changes client-side channel dispatch and receive-window accounting
  to isolate slow channels; details and the tested boundary are recorded in
  `vendor/russh/CAPSTAN_PATCH.md`. The agent client also omits upstream debug
  statements that logged identity lists and signing payloads.

## Frontend dependency license exceptions

- `lru-cache` uses Blue Oak Model License 1.0.0.
- `caniuse-lite` contains browser support data under CC-BY-4.0.
- `@csstools/color-helpers` uses MIT-0.
- These transitive packages are present in `ui/pnpm-lock.yaml`. Their SPDX
  identifiers are explicitly allowlisted by `scripts/Check-Licenses.ps1`; the
  check reports package counts and fails on missing or unreviewed metadata.

## ssh-key 0.7.0-rc.10 (Capstan RSA backend patch)

- License: Apache-2.0 OR MIT; both texts are retained in `vendor/ssh-key`.
- Source: RustCrypto/SSH revision `c8605550cb31f750c3bf318eea433141af84f2c7`.
- RSA key validation, generation and private signing use AWS-LC rather than the
  RustCrypto `rsa` package. See `vendor/ssh-key/CAPSTAN_PATCH.md`.
- `aws-lc-rs` 1.18.1: ISC AND (Apache-2.0 OR ISC), upstream
  https://github.com/aws/aws-lc-rs . Its pinned `aws-lc-sys` source package carries
  the ISC, Apache-2.0, MIT, MIT-0 and BSD-3-Clause component notices.
- crypto-bigint 0.7.6: Apache-2.0 OR MIT, used for bounded-width local key import;
  it does not perform RSA private signing in Capstan.
- The former RUSTSEC-2023-0071 dependency exception is removed. Release audit
  still checks the complete resolved dependency graph; this replacement does
  not imply that all other milestone acceptance items are complete.
