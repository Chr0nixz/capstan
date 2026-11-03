# xterm 5.4.0 hidden selection redraw

`@xterm__xterm@5.4.0.patch` keeps the pinned upstream version and MIT license.
It changes only `RenderService.handleSelectionChanged`: a paused renderer saves
the latest selection and requests a full refresh on resume. Parsing, scrollback,
selection state, terminal output and ACKs continue while the pane is hidden.

Without this guard, selection/scroll events bypass xterm's normal paused-row
rendering path. The hidden DOM fallback repeatedly measures zero-width glyphs
and rebuilds rows. A real WebView2 CPU profile identified this path during M1
input latency diagnosis. No application visibility or acceptance limit changes.

The patch includes the corresponding published JavaScript and corrected source
map. pnpm records its content hash in the frozen lockfile. Run
`pnpm -C ui install --offline --frozen-lockfile` to apply it reproducibly and
`pnpm -C ui test:security` for the real-browser regression: hidden output and
selection changes do not mutate rendered rows, logical selection survives,
showing the pane restores it, and a selection cleared while hidden stays clear.

Reassess this patch before upgrading xterm; do not carry it blindly to a newer
version. Profiling failures and before/after measurements are retained under
`docs/agent/baselines/M2/raw/2026-10-21-quantitative/`.
