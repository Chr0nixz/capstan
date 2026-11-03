---
name: Capstan
description: A matte keycap-inspired visual system for a Windows-first terminal and SSH workspace.
colors:
  signal-amber: "#d9a441"
  signal-amber-bright: "#e8bf64"
  signal-amber-hover: "#e6b555"
  signal-amber-pressed: "#c99536"
  amber-ink: "#1a1408"
  amber-wash: "rgb(217 164 65 / 0.12)"
  amber-line: "rgb(217 164 65 / 0.42)"
  safety-red: "#d0605e"
  safety-red-text: "#f2a6a2"
  red-wash: "rgb(208 96 94 / 0.12)"
  red-line: "rgb(208 96 94 / 0.45)"
  quiet-green: "#8ea981"
  quiet-green-text: "#b3c9a7"
  green-wash: "rgb(142 169 129 / 0.12)"
  graphite-frame: "#121316"
  graphite-canvas: "#16171a"
  graphite-chrome: "#1d1f23"
  graphite-panel: "#212328"
  keycap-face: "#26282d"
  keycap-face-hover: "#2d3036"
  keycap-face-pressed: "#222429"
  graphite-selected: "#2b2e34"
  keycap-edge: "#3a3e45"
  keycap-edge-strong: "#4c515a"
  quiet-divider: "#2f3238"
  soft-divider: "#26292e"
  warm-legend: "#d8d4cb"
  bright-legend: "#ece8df"
  muted-legend: "#9a9ea6"
  faint-legend: "#868b93"
  graphite-scrim: "rgb(8 9 11 / 0.62)"
  terminal-legend: "#e4e0d7"
  terminal-selection: "#4a3f28"
  paper-frame: "#e9e5de"
  paper-canvas: "#f6f4ef"
  paper-chrome: "#efece6"
  paper-panel: "#fbfaf7"
  paper-well: "#ffffff"
  paper-face-hover: "#f3f0ea"
  paper-face-pressed: "#e9e5dd"
  paper-selected: "#e6e1d6"
  paper-edge: "#c9c3b7"
  paper-edge-strong: "#aea799"
  paper-divider: "#dcd7cd"
  paper-soft-divider: "#e6e2da"
  ink-legend: "#2b2c2f"
  ink-legend-strong: "#17181a"
  ink-muted: "#5d6067"
  ink-faint: "#686b72"
  paper-amber: "#8a5d0c"
  paper-amber-strong: "#6f4a08"
  paper-amber-wash: "rgb(217 164 65 / 0.16)"
  paper-amber-line: "rgb(138 93 12 / 0.45)"
  paper-red: "#b13f3c"
  paper-red-text: "#9e2f2c"
  paper-red-wash: "rgb(177 63 60 / 0.09)"
  paper-red-line: "rgb(177 63 60 / 0.4)"
  paper-green: "#4d7a40"
  paper-green-text: "#3f6a33"
  paper-green-wash: "rgb(77 122 64 / 0.1)"
  paper-scrim: "rgb(30 28 24 / 0.32)"
  paper-terminal: "#f7f8fa"
typography:
  display:
    fontFamily: "Segoe UI Variable Display, Segoe UI Variable, Segoe UI, Microsoft YaHei UI, sans-serif"
    fontSize: "18px"
    fontWeight: 600
  headline:
    fontFamily: "Segoe UI Variable Display, Segoe UI Variable, Segoe UI, Microsoft YaHei UI, sans-serif"
    fontSize: "15px"
    fontWeight: 600
  title:
    fontFamily: "Segoe UI Variable Display, Segoe UI Variable, Segoe UI, Microsoft YaHei UI, sans-serif"
    fontSize: "13px"
    fontWeight: 600
  title-compact:
    fontFamily: "Segoe UI Variable Text, Segoe UI Variable, Segoe UI, Microsoft YaHei UI, sans-serif"
    fontSize: "12.5px"
    fontWeight: 600
  body:
    fontFamily: "Segoe UI Variable Text, Segoe UI Variable, Segoe UI, Microsoft YaHei UI, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.5
  body-compact:
    fontFamily: "Segoe UI Variable Text, Segoe UI Variable, Segoe UI, Microsoft YaHei UI, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.45
  label:
    fontFamily: "Segoe UI Variable Text, Segoe UI Variable, Segoe UI, Microsoft YaHei UI, sans-serif"
    fontSize: "11.5px"
    fontWeight: 600
    letterSpacing: "0.02em"
  caption:
    fontFamily: "Segoe UI Variable Text, Segoe UI Variable, Segoe UI, Microsoft YaHei UI, sans-serif"
    fontSize: "11px"
    fontWeight: 400
    fontFeature: "\"tnum\""
  mono:
    fontFamily: "Cascadia Mono, Sarasa Mono SC, Consolas, Microsoft YaHei UI, monospace"
    fontSize: "12px"
    fontWeight: 400
  mono-small:
    fontFamily: "Cascadia Mono, Sarasa Mono SC, Consolas, Microsoft YaHei UI, monospace"
    fontSize: "11px"
    fontWeight: 400
  terminal:
    fontFamily: "Cascadia Mono, Microsoft YaHei UI, Consolas, monospace"
    fontSize: "15px"
    lineHeight: 1
rounded:
  sm: "3px"
  md: "4px"
  keycap: "6px"
  lg: "7px"
  keycap-lg: "9px"
  dialog: "10px"
  pill: "999px"
spacing:
  xs: "4px"
  sm: "6px"
  md: "8px"
  lg: "12px"
  xl: "16px"
  2xl: "22px"
components:
  button-keycap:
    backgroundColor: "{colors.keycap-face}"
    textColor: "{colors.warm-legend}"
    typography: "{typography.body-compact}"
    rounded: "{rounded.md}"
    padding: "0 10px"
    height: "28px"
  button-keycap-hover:
    backgroundColor: "{colors.keycap-face-hover}"
    textColor: "{colors.bright-legend}"
  button-keycap-pressed:
    backgroundColor: "{colors.keycap-face-pressed}"
  button-primary:
    backgroundColor: "{colors.signal-amber}"
    textColor: "{colors.amber-ink}"
    typography: "{typography.title-compact}"
    rounded: "{rounded.md}"
    padding: "0 14px"
    height: "32px"
  button-primary-hover:
    backgroundColor: "{colors.signal-amber-hover}"
  button-primary-pressed:
    backgroundColor: "{colors.signal-amber-pressed}"
  button-danger:
    backgroundColor: "{colors.red-wash}"
    textColor: "{colors.safety-red-text}"
    rounded: "{rounded.md}"
    height: "28px"
  button-ghost:
    backgroundColor: "transparent"
    textColor: "{colors.muted-legend}"
    rounded: "{rounded.md}"
    height: "28px"
  button-ghost-hover:
    backgroundColor: "{colors.keycap-face-hover}"
  button-icon:
    backgroundColor: "transparent"
    textColor: "{colors.muted-legend}"
    rounded: "{rounded.md}"
    size: "28px"
  button-toggle-on:
    backgroundColor: "{colors.amber-wash}"
    textColor: "{colors.signal-amber-bright}"
  input:
    backgroundColor: "{colors.graphite-canvas}"
    textColor: "{colors.bright-legend}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 9px"
    height: "30px"
  tab:
    backgroundColor: "{colors.graphite-chrome}"
    textColor: "{colors.muted-legend}"
    padding: "0 4px 0 12px"
    height: "34px"
  tab-active:
    backgroundColor: "{colors.graphite-canvas}"
    textColor: "{colors.bright-legend}"
  host-row:
    backgroundColor: "transparent"
    textColor: "{colors.warm-legend}"
    rounded: "{rounded.lg}"
    padding: "6px 8px"
    height: "46px"
  host-row-hover:
    backgroundColor: "{colors.keycap-face}"
  host-key:
    backgroundColor: "{colors.keycap-face}"
    textColor: "{colors.bright-legend}"
    typography: "{typography.title-compact}"
    rounded: "{rounded.keycap}"
    size: "30px"
  host-key-production:
    textColor: "{colors.safety-red-text}"
  production-chip:
    backgroundColor: "{colors.red-wash}"
    textColor: "{colors.safety-red-text}"
    typography: "{typography.caption}"
    rounded: "{rounded.sm}"
    padding: "0 5px"
  segmented-tab-selected:
    backgroundColor: "{colors.keycap-face}"
    textColor: "{colors.bright-legend}"
    rounded: "{rounded.md}"
    height: "26px"
  count-badge:
    backgroundColor: "{colors.keycap-face}"
    textColor: "{colors.muted-legend}"
    typography: "{typography.caption}"
    rounded: "{rounded.pill}"
    height: "18px"
  count-badge-active:
    backgroundColor: "{colors.signal-amber}"
    textColor: "{colors.amber-ink}"
  chip:
    backgroundColor: "transparent"
    textColor: "{colors.muted-legend}"
    typography: "{typography.caption}"
    rounded: "{rounded.pill}"
    padding: "1px 7px"
  led:
    backgroundColor: "{colors.faint-legend}"
    rounded: "{rounded.pill}"
    size: "7px"
  led-connecting:
    backgroundColor: "{colors.signal-amber}"
  led-live:
    backgroundColor: "{colors.quiet-green}"
  led-failed:
    backgroundColor: "{colors.safety-red}"
  sheet:
    backgroundColor: "{colors.graphite-panel}"
    textColor: "{colors.warm-legend}"
    width: "460px"
  dialog:
    backgroundColor: "{colors.graphite-panel}"
    textColor: "{colors.warm-legend}"
    rounded: "{rounded.dialog}"
    padding: "22px"
    width: "520px"
  file-row:
    textColor: "{colors.muted-legend}"
    typography: "{typography.body-compact}"
    height: "28px"
  file-row-selected:
    backgroundColor: "{colors.graphite-selected}"
  inline-error:
    backgroundColor: "{colors.red-wash}"
    textColor: "{colors.safety-red-text}"
    typography: "{typography.body-compact}"
    rounded: "{rounded.lg}"
    padding: "8px 6px 8px 10px"
  inline-notice:
    backgroundColor: "{colors.green-wash}"
    textColor: "{colors.quiet-green-text}"
  menu:
    backgroundColor: "{colors.graphite-panel}"
    textColor: "{colors.warm-legend}"
    rounded: "{rounded.lg}"
    padding: "4px"
    width: "228px"
  menu-item:
    textColor: "{colors.warm-legend}"
    typography: "{typography.body-compact}"
    rounded: "{rounded.md}"
    padding: "0 8px"
    height: "28px"
  menu-item-hover:
    backgroundColor: "{colors.keycap-face-hover}"
    textColor: "{colors.bright-legend}"
  session-bar:
    backgroundColor: "{colors.graphite-panel}"
    textColor: "{colors.warm-legend}"
    typography: "{typography.body-compact}"
    rounded: "{rounded.lg}"
    padding: "7px 7px 7px 12px"
  field-error:
    textColor: "{colors.safety-red-text}"
    typography: "{typography.label}"
  count-badge-attention:
    backgroundColor: "{colors.red-wash}"
    textColor: "{colors.safety-red-text}"
    typography: "{typography.caption}"
    rounded: "{rounded.pill}"
    height: "18px"
  follow-toggle-on:
    backgroundColor: "{colors.keycap-face}"
    textColor: "{colors.warm-legend}"
    typography: "{typography.label}"
    rounded: "{rounded.pill}"
    height: "24px"
  keyboard-key:
    backgroundColor: "{colors.keycap-face}"
    textColor: "{colors.warm-legend}"
    typography: "{typography.mono-small}"
    rounded: "{rounded.md}"
    padding: "1px 6px"
---

# Design System: Capstan

## Overview

**Creative North Star: "Capstan Keycap Colorway"**

Capstan treats its interface like a set of matte mechanical keycaps. The terminal is the alpha key: quiet, legible, flush to the window, and always in front. Utility surfaces sit one tonal step above it, controls are small raised keys with a one-pixel highlight, and a single amber accent key marks whatever is active, selected, or in progress. The material is matte PBT, not gloss: dense, durable, and calm in a dim room.

The window shell is native Windows (system type, Mica behind a transparent root, Windows caption buttons); the workspace keeps its own character through a strict graphite ladder, warm legends, and a small signal vocabulary. State follows the dark-cockpit principle: a healthy session is quiet, while connecting, failed, and host-key-changed states light up in amber and red. A host can switch its own workspace to the light Paper colorway without changing the chrome around it.

Motion confirms state and steps aside: keys press 1px in 80ms, state changes take 110–260ms on one strong ease-out curve, and loops (breathing lights, the busy bar, the transfer sheen and scan line) run on ease-in-out or linear timing, each with a reduced-motion fallback.

**Key Characteristics:**
- Matte graphite layers stepped in small luminance increments, with warm, low-saturation legends.
- The terminal is flush and borderless; its output is the brightest, densest content on screen.
- One amber accent key: active tab, primary action, focus ring, toggled tools, running work.
- Keycap tactility: a one-pixel top highlight, a one-pixel bottom shade, and a one-pixel press.
- Signal lights carry connection and transfer state: quiet when healthy, unmistakable when not.

## Colors

A low-luminance neutral ladder with warm legends, one amber accent, and two operational signals; the Paper colorway mirrors every role in a warm light set.

### Primary
- **Signal Amber** (signal-amber): The one accent key: active-tab key, primary actions, focus ring, caret, selected-row edge, toggled tools, the Favorites star, connecting lights, running progress, and the active transfer badge. Signal Amber Bright is its text grade on dark surfaces; Amber Ink sits on amber fills (8.1:1).
- **Amber Wash and Line** (amber-wash, amber-line): A 12% tint and a 42% hairline for toggled and drop-target state, so a control never becomes a solid amber block.

### Secondary
- **Safety Red** (safety-red): Production hosts, failures, interrupted transfers, destructive actions, invalid fields, and the changed-host-key alert. As text it uses Safety Red Text (7.6:1 on Keycap Face).
- **Quiet Green** (quiet-green): Live connections, completed transfers, symlinks, and confirmations; less saturated than consumer status green. Quiet Green Text carries success copy.

### Neutral
- **Graphite ladder** (graphite-frame, graphite-canvas, graphite-chrome, graphite-panel): Frame is the title bar; Canvas is the workspace, active tab, input wells, and terminal; Chrome carries the sidebar, tab strip, remote-files panel, queues, and settings groups; Panel is only for things that float.
- **Keycap Face** (keycap-face, with hover and pressed steps): Raised controls, monogram keys, the selected segment, and row hover. Graphite Selected marks selected file rows.
- **Edges and dividers** (keycap-edge, keycap-edge-strong, quiet-divider, soft-divider): One-pixel control outlines (strong on hover), structural separators, and row separators.
- **Legends** (warm-legend, bright-legend, muted-legend, faint-legend): Warm is primary text (11.2:1 on Chrome); Bright is for headings, hovered controls, active tabs, and field values; Muted is for secondary labels and resting icons (6.1:1); Faint is for group labels, table headers, and placeholders (4.8:1) and is the floor for any text.
- **Terminal** (terminal-legend, terminal-selection): A warm xterm foreground and an amber-brown selection; the cursor is Signal Amber.

### Paper Colorway
The light set maps role for role, with Paper Amber, Red, and Green darkened to text grade (4.8–5.5:1) and Ink Faint above 4.5:1 on Paper Chrome. Paper Well is the one pure white, used only inside fields. When an SSH host uses the light terminal theme, its terminal, remote-files panel, queues, and file dialogs switch to Paper; the title bar, sidebar, tab strip, and sheets stay graphite.

### Named Rules
**The One Accent Key Rule.** Amber marks state only: active, selected, focused, toggled, or in progress. Never use it as a panel wash, a decorative border, a destructive action, or a heading color.

**The Production Red Rule.** Host identity has one color. A production host gets key-sized Safety Red marks (its monogram key, its tab key and icon, a small chip, the title-bar badge), never a tinted panel, and the chip follows the host into every surface that can change it. Broadcast never borrows red: it is a toggled tool, so it speaks amber.

**The Night Vision Rule.** Chrome avoids saturated blue and extreme black or white; contrast comes from deliberate luminance steps.

**The Wash-and-Line Rule.** Signal states on controls and messages are a 9–16% wash plus a 40–45% hairline in the signal color, with text-grade signal text on top. Solid signal fills are reserved for the primary action, the active transfer badge, signal lights, and progress bars.

## Typography

**Display Font:** Segoe UI Variable Display (with Segoe UI Variable, Segoe UI, Microsoft YaHei UI fallback)
**Body Font:** Segoe UI Variable Text (with the same fallback chain)
**Label/Mono Font:** Cascadia Mono (with Sarasa Mono SC, Consolas, Microsoft YaHei UI fallback)

**Character:** Compact, neutral, and native to Windows; the Display optical size is reserved for headings and monograms. Chinese falls back to Microsoft YaHei UI in both stacks.

### Hierarchy
- **Display** (600, 18px): Safety and confirmation dialog titles.
- **Headline** (600, 15px): The locked-vault title and settings section titles.
- **Title** (600, 13px): Sheet headers and the brand mark.
- **Title Compact** (600, 12.5px): Panel titles, list item names, and primary button labels; tab labels use it at 400.
- **Body** (400, 13px, 1.5): Field values, host names (500), and dialog copy.
- **Body Compact** (400, 12px, 1.45): The working size: buttons, field labels, messages, file rows, and transfer details.
- **Label** (600, 11.5px, 0.02em): Group and section headings in sentence case, never uppercase; field errors use it at 400.
- **Caption** (400, 11px, tabular figures): Table headers, footers, counts, badges, and chips.
- **Mono** (12px) and **Mono Small** (11px): Paths and fingerprints; host addresses, modes, and transfer paths.
- **Terminal** (Cascadia Mono, 15px, line-height 1): The xterm default; it preserves cell alignment and CJK width.

The half-pixel steps (11.5px, 12.5px) sit very close to their neighbors. New text picks one of these roles instead of adding a size.

### Named Rules
**The Context Before Decoration Rule.** Every pane names its scope in monospace (the title-bar context, the remote-files host label, the path field) before any decorative label earns space.

**The Monospace Is Data Rule.** Monospace never styles a label, button, or heading. If the user could not paste the text into a shell, it is set in Segoe UI.

**The Tabular Figures Rule.** Counts, sizes, progress, and search results use tabular figures so live numbers update without jitter.

## Layout

A 36px title bar sits over two columns: a 272px host sidebar and a flexible terminal area. The sidebar heading, tab strip, sheet headers, and remote-files header share one 34px row, so their bottom edges form a single line. The terminal sits flush on Canvas (10px top, 6px right and bottom, 14px left padding) with no card or border.

The remote-files panel docks right (36% default, 280px minimum) or bottom (42% default), resizes from its leading edge between 20% and 60%, and collapses to a 34px strip without remounting the terminal. The transfer and remote-edit queues stack under the terminal behind 32px headings. The sidebar can hide at any width except while the vault is locked, because unlocking lives there; host editing and vault management open in a 460px sheet from its edge.

Compaction follows the workspace, not the window. Under a 760px workspace the remote-files panel starts collapsed and a right-docked panel stacks under the terminal. Inside the panel, columns yield before names truncate: the title hides under 540px, Modified under 420px, Type and mode under 340px. Under a 720px window the sidebar starts closed as an overlay and sheets take the full width.

Spacing follows an observed 4, 6, 8, 12, 16, 22px rhythm: 4–6px inside control clusters, 8–12px between rows and in gutters, 16px in sheets, 22px in dialogs. Controls are 28px, fields 30px, footer and dialog actions 32px, file rows 28px, host rows 46px.

Not built yet: focus mode and the server status bar. The status bar will be a row of signal lights on Chrome at the 34px row height. Snippets and tunnels open as sheets from two keys in the tab strip, so PLAN's activity bar is not needed and the One Sidebar Job Rule holds.

### Named Rules
**The Terminal First Rule.** The first viewport is the active terminal and its context, never an onboarding hero, metric dashboard, or card gallery.

**The Shared Row Rule.** Chrome rows are 34px and line up across the sidebar, tab strip, sheets, and remote-files panel; a new header row matches that height.

**The Stable Row Rule.** Status changes recolor or update an existing row in place; they never reorder or shift nearby rows while the user is reading.

**The One Sidebar Job Rule.** The sidebar finds and opens hosts. Settings, keys, credentials, and groups live in the Settings sheet.

**The Place, Not Spawner Rule.** A host is a place: opening it focuses its existing tab and reconnects in place if the session ended; a second tab for the same host takes an explicit gesture.

**The Way In, Way Out Rule.** Every pane is reachable from the keyboard while the terminal has focus, and Esc returns to the terminal; Settings › Shortcuts is the key map.

## Elevation & Depth

Depth comes from tonal layering first, along the graphite ladder; shadows appear only where a surface truly sits above another. Keycaps get their relief from inset hairlines, and a pressed key loses it. Mica belongs to the native window shell and is never repeated as an in-workspace blur.

### Shadow Vocabulary
- **Key Top** (`box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.05)`; Paper: `rgb(255 255 255 / 0.8)`): The highlight on every raised key.
- **Key Bottom** (`box-shadow: inset 0 -1px 0 rgb(0 0 0 / 0.28)`; Paper: `rgb(60 50 30 / 0.08)`): The matching shade; large keys (host monogram, transfer direction, locked-vault key) deepen it to 2–3px.
- **Primary Key** (`box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.28), inset 0 -1px 0 rgb(0 0 0 / 0.18)`): The relief on amber keys.
- **Pop** (`box-shadow: 0 18px 48px rgb(0 0 0 / 0.5), 0 2px 8px rgb(0 0 0 / 0.32)`): Dialogs, popovers, and the terminal search bar.
- **Sheet** (`box-shadow: 18px 0 44px rgb(0 0 0 / 0.38)`): Sheets and the narrow-window sidebar overlay.
- **LED Cutout** (`box-shadow: 0 0 0 2px <surface>`): A ring in the underlying surface color that separates a signal light from the key it sits on.

### Named Rules
**The Flat Surface Rule.** Use tonal separation and one quiet divider before reaching for a shadow. Resting surfaces are flat; only floating layers cast shadows.

**The Signal Light Rule.** Brightness belongs to state. A visible increase in luminance means a connection, transfer, safety condition, or focused action needs attention.

## Shapes

Square-first with gently softened corners: 3–4px for small controls and fields; 7px for rows, settings groups, messages, and popovers; 6px and 9px for the large keys so they read as keycaps; 10px for dialogs. Fully round shapes are reserved for signal lights, count badges, chips, and the Follow toggle.

Borders are one-pixel hairlines in Keycap Edge (controls) or Quiet Divider (structure). Focus is one 2px Signal Amber outline with no glow, offset 1px outside or inset 2–3px inside tight rows. Icons are authored line icons on a 16px grid with a 1.4px stroke and round caps and joins, in currentColor; caption buttons use 1px glyphs.

## Components

Sizes, fills, and radii for each component are in the frontmatter; this section covers relief, states, and composition.

### Buttons
Every button is a small keycap: tactile, quiet, and confident.
- **Keycap:** A Keycap Edge hairline with Key Top and Key Bottom relief. Hover lifts to the hover face, strong edge, and Bright Legend; press drops 1px and loses the relief.
- **Primary:** Solid amber with Amber Ink at 600; hover brightens and press darkens without changing size. Sheet-footer and dialog actions are 32px.
- **Danger:** Red wash-and-line with Safety Red Text and no relief; its label names the action and its object ("Delete file").
- **Others:** Ghost is used for Cancel and Reject. Icon keys always carry a tooltip and a visually hidden label. Toggle on is Amber Wash with a 1px Amber Line inset ring. Disabled is 50% opacity (32% in dense toolbars) with no relief.

### Inputs / Fields
- A recessed well with a Keycap Edge hairline, a faint inner top shadow, Bright Legend values, and an amber caret. Focus turns the border amber with a 1px ring and no glow; invalid turns it Safety Red with an error line in Label size beneath.
- Selects, checkboxes, and ranges stay native: the select chevron is two CSS gradients because the CSP forbids `data:` images, and checks use amber `accent-color`.

### Terminal tabs
- Tabs are 188px, shrinking to 112px before the strip scrolls, with short dividers that hide beside the active or hovered tab. Each carries a kind icon with its signal light, an ellipsized label, and a close key on hover or when active; new tabs rise 5px and fade in.
- The active tab takes its terminal's background so it merges with the pane, and a 2px amber key slides open along its top (scaleX, 260ms). A production tab's key and icon are Safety Red, and the key stays at 55% when the tab is inactive.

### Host list (signature)
- Rows hold a monogram key, the name (Body at 500), and `user@address:port` in Mono Small. Hover or focus raises the row to Keycap Face and reveals an edit key; live hosts carry a 6% green tint. Group labels carry tabular counts, and Favorites come first with an amber star.
- The monogram key is a 30px keycap with a one-step face gradient, a 2px bottom shade, a two-character Display legend (one glyph for CJK names), and the host's signal light at its corner.
- Production hosts get a Safety Red legend, edge, and bottom shade, a "Production" chip after the name, and a red badge before the title-bar context while active.

### Signal lights
- 7px circles behind an LED Cutout ring, identical on the tab and the monogram key. Connecting breathes amber to 30% opacity every 1.4s; Live is solid Quiet Green; Failed is solid Safety Red; Ended is a hollow 1.5px Faint Legend ring.

### Session-ended bar
- A Panel bar centered 12px above the bottom of an ended terminal (Pop shadow, Safety Red edge on failure) with the tab's light, the production chip, the status, an amber Reconnect key, an "or press Enter" hint, and a ghost Close.

### Sheets
- A 460px Panel column that slides 28px in from the sidebar edge (260ms), with the Sheet shadow over a 38% scrim that leaves the sidebar visible: a 34px header, a 16px-padded body, and a pinned footer of 32px actions, submit first.
- Settings tabs are a segmented control on a Canvas track; the selected segment is a raised keycap with a 2px amber bottom key, so the focus ring never out-shouts it. Shortcut keys are drawn as 11px mono keycaps.

### Dialogs
- Panel with the Pop shadow, rising 10px from 98% scale (260ms) over a 62% scrim. Actions are 32px and right-aligned, and focus opens on the safe action (Cancel, Reject, or Close and keep blocked).
- **Host key:** A 32px mark beside the Display title (amber shield for a new key; red alert and a red border for a changed one); the saved host and `target:port` with the production chip; fingerprints in 12px mono four-character groups on a Canvas well, where groups that differ from the trusted key take a red wash; and the `ssh-keygen -lf` command in a mono well with a copy key.
- **Confirmation:** One alert dialog for every irreversible action: a mark (Safety Red when destructive), a question title naming the object and host, a one-sentence consequence, the host line with its chip, the path in a mono well, a ghost Cancel, and a danger key. Remote renames and permission changes use the same layout with the amber primary.

### Remote-files panel
- **Header:** A folder icon, the title, the production chip, and the host in faint Mono Small; three file keys plus More; then the dock and collapse keys after a hairline.
- **Splitter:** A 5px edge whose 2px line shows Keycap Edge Strong on hover and amber while focused or dragged.
- **Path row:** Parent, Refresh, a mono path field with an inset go key, and the Follow toggle at its end. Follow is quiet when on (a raised face with a Quiet Green dot), breathes amber only while waiting for a directory hint, and its dot goes hollow once the session ends. The server-hostname editor takes a row only while Follow waits, while it is open, or when it reports an error; the More menu opens it at any time.
- **Menus:** Panel popovers of 28px items with icons, separators, key hints, and Delete in Safety Red Text.
- **Table:** A sticky Caption header, right-aligned tabular sizes, and middle-truncated names that keep the extension. Directory icons are Muted Legend, symlinks green, files faint. Hover adds a 3% tint; selection adds a 2px amber inner edge on the first cell.
- **Busy, footer, ended:** A 2px amber sweep under the controls while busy, a Caption footer for the drop hint and pagination, and a quiet Keycap Face note once the session behind the panel has ended.

### Transfer and remote-edit queues
- A collapsible 32px heading with a chevron, a count badge, and a one-line summary ("2 in progress · 38s remaining"). The badge is solid amber while work runs, with a 1px amber scan line on the queue's top edge; when anything needs a decision, badge and summary turn Safety Red wash-and-line and the queue opens.
- Rows carry a direction keycap colored by state and a 4px progress bar: amber with a sheen while running, Quiet Green Text when complete, Safety Red Text when failed or interrupted. In a conflict, Skip is ghost, Replace is danger, and Save as is primary.

### Command palette
- A Panel popover 76px from the top, under the tab strip, with no scrim, so the workspace stays visible. A 44px search row, a scrolling list, and a 30px Chrome hint row of keycaps (↑ ↓, Enter, Esc).
- Rows are 30px menu items, not keycaps: a faint kind icon, the label, a production chip, faint mono detail, and a shortcut keycap on the right. The active row takes the hover face. Empty search shows sections (Recent, Hosts, Snippets, Local shells, Workspace, Panels, Terminal theme); typing ranks one flat list, label prefixes first.
- Focus stays in the search field; the active row is announced through `aria-activedescendant` and kept in view.

### Split panes and broadcast
- Panes get a 28px heading row (it sits inside the terminal area, so it stays below the 34px chrome row): a signal light, the pane name, and the production chip. The focused pane's heading carries a 2px amber top key.
- Broadcast is a toggled tool: the toggle key takes Amber Wash and Line, selected panes take a 2px dashed Signal Amber frame and a Broadcast chip, and a wash-and-line status bar names the target count. The first line break to a target set asks once; several lines at once always ask.

### Messages, chips, and badges
- Inline errors and notices are 7px wash-and-line strips with a leading icon and a dismiss key, rising 4px into place. Success notices clear themselves after 6 seconds; errors stay until dismissed. Chips are Caption pill outlines in Quiet Divider; count badges are 18px pills on Keycap Face.

### Title bar
- A 36px Frame band with the brand, a divider, and the context (mono for `user@host`), plus 46px Windows-convention caption buttons. Close turns system red (#c42b1c) with white glyphs on hover.

## Do's and Don'ts

### Do:
- **Do** keep terminal output the most legible, continuous surface: flush to the tab strip, on Canvas, with no surrounding card.
- **Do** land focus on the safe action in every safety or confirmation dialog.
- **Do** give every icon-only control a tooltip and a visually hidden label, and draw icons from the authored 16px, 1.4px-stroke set.
- **Do** animate only transform and opacity, and give every loop a reduced-motion fallback.
- **Do** respect forced-colors mode: system-colored borders, Highlight for the active tab and selection, and outlined signal lights.
- **Do** keep terminal themes independent from the chrome so imported Windows Terminal and iTerm2 themes remain usable.
- **Do** give the terminal its rows back in small spaces: a collapsed file panel under a 760px workspace, one-line transfer progress, and a closed sidebar under a 720px window.

### Don't:
- **Don't** use cyberpunk neon, RGB gaming color, glowing borders, or saturated blue as the brand voice.
- **Don't** reproduce old enterprise terminal chrome: noisy toolbars, dense unlabeled controls, gray-on-gray clutter.
- **Don't** build the workspace from floating rounded cards, decorative gradients, or nested containers that compete with the terminal.
- **Don't** add ropes, anchors, ports, or other literal nautical illustrations as ornamental UI language.
- **Don't** use Unicode glyphs or emoji as interface icons, or `data:` URIs for interface graphics.
- **Don't** animate width, height, or other layout properties.
- **Don't** offer any way to accept a changed host key from its alert dialog.
- **Don't** confirm with native `window.confirm`, or style a destructive confirmation with the amber primary.
- **Don't** add more than three always-visible file keys to the remote-files header; new file actions go into its menus.
- **Don't** let a single keystroke reconnect or change a production host; on a production host the first Enter only focuses Reconnect.
- **Don't** light amber for a healthy default such as Follow on or a finished transfer.
