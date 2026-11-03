# Capstan window adapter

Upstream: `tauri-plugin-snap-layout` **1.0.9**, MIT,
[Hyph-M/tauri-plugin-snap-layout](https://github.com/Hyph-M/tauri-plugin-snap-layout).
The original copyright notices and full license are retained in `LICENSE`.

This narrowed fork retains the native child HWND / `HTMAXBUTTON` technique.
It is a Tauri window adapter outside the domain crates. Application and domain
code retain `unsafe_code = deny`; FFI remains behind the adapter's safe API.

Changes from the published crate:

- Remove script injection, CSS rewriting, general builder options and global
  attach APIs. React owns geometry, listeners, DPI/zoom and teardown.
- Restrict both commands to `main`; bound geometry to a single titlebar button.
  Scale measured fractional DOM edges by devicePixelRatio and check client bounds.
- Complete owner-thread work before replying; report stable errors instead of
  silently accepting failed window creation, property installation or positioning.
- Use a NULL_BRUSH class, suppress painting, and position the child above WebView2
  siblings. Preserve DOM keyboard operation and forward native mouse activation.
- Pair every transferred Box with destruction or failed-install cleanup. Ignore
  stray mouse-up, release hover/press state when leaving, and remove the parent's
  property when the child is destroyed.
- Serialize frontend ownership across StrictMode remounts so old cleanup cannot
  destroy a replacement hitbox. Dispose subscriptions received after unmount.
- Pin dependencies and include geometry plus real hidden-window message tests.

Run `cargo test --manifest-path vendor/tauri-plugin-snap-layout/Cargo.toml --offline`
and the corresponding fmt/clippy checks when upgrading. These tests cannot prove
the Windows 11 flyout, actual screen painting, accessibility or mixed-monitor DPI
behavior; those remain maintainer acceptance items. No ConPTY binary is included.
# M2 dependency maintenance

Tokio is pinned to 1.53.3 for RUSTSEC-2026-0386, matching the workspace.
This MIT-licensed patch update changes no native hit-test behavior.

