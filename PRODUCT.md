# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

Tauri 2 desktop app rendered in WebView2. Windows 11 first; macOS/Linux later. Window chrome (title bar, Snap Layouts, Mica) follows native Windows behavior.

## Stack

Tauri 2 + Rust backend; React 19 + TypeScript + Vite + Tailwind 4 + Radix UI; xterm.js for terminal rendering. Full plan in docs/PLAN.md.

## Users

One primary user: the maintainer, a developer who manages a small fleet of Linux servers (fewer than 20 hosts, typically 2–5 tabs open at once) plus local PowerShell and WSL shells. Uses it mostly at night or in dim light for long sessions. Secondary audience: GitHub visitors evaluating it as an open-source tool.

## Product Purpose

A modern, lightweight replacement for MobaXterm on Windows: SSH session manager, SFTP side panel that follows the terminal's current directory, local shells, split panes with broadcast input, port forwarding, and command snippets. Success means the maintainer uses it every day instead of MobaXterm/Tabby without friction.

## Positioning

Windows as a first-class citizen (bundled modern ConPTY, deep WSL integration, Windows Hello unlock, native Snap Layouts and Mica, system SSH agent), combined with MobaXterm's workflow (one SSH connection carries terminal, SFTP, status, and tunnels) without bundling Chromium or Java. Not cross-platform-first, no cloud sync, no team features, no AI.

## Operating Context

Long sessions in dim light; terminal output is the primary content and must stay the most legible thing on screen. The user frequently switches between a few hosts, watches logs, edits remote files in an external editor, and transfers files. Keyboard-first; a command palette is planned for M3.

## Capabilities and Constraints

- Built: collapsible host sidebar, terminal tabs, dockable remote-files panel, and transfer queue. Planned: split panes and a command palette (Ctrl+Shift+P) in M3, a server status bar in M4. PLAN's activity bar is still open: it conflicts with the One Sidebar Job Rule in DESIGN.md, so where tunnels and snippets live is decided in M3.
- Dark graphite chrome with a Win11 Mica backdrop; a host can switch its own workspace to the light Paper colorway. Following the system light/dark setting (PLAN §4.11) is not built.
- UI bilingual (zh-CN, en); CJK text and input method must render correctly.
- Terminal color schemes importable from Windows Terminal and iTerm2.
- Credentials never reach the frontend.

## Brand Commitments

- Name: Capstan (a dockside winch that hauls in mooring lines; "terminal" also means a port).
- License GPL-3.0-or-later.
- Reference point named by the user: Tabby's design is "decent but not good enough" — the bar to clear.
- Explicit anti-references: old-school enterprise tool chrome (gray control clutter, noisy toolbars like Xshell/MobaXterm), and cyberpunk neon / glowing hacker aesthetics.
- Native-feel recommendation accepted: Windows-native skeleton with its own recognizable character.

## Evidence on Hand

No logo or screenshots exist yet; the app icon is a generated M0 placeholder. No users besides the maintainer; no benchmarks or testimonials — none may be invented.

## Product Principles

1. Terminal output is the hero; chrome recedes.
2. Native Windows behavior over custom reinvention.
3. Every feature must earn daily use for one person; no feature bloat.
4. Safety is visible: production hosts, broadcast mode, and host-key changes are unmistakable.
5. Lightweight in size, memory, and visual noise.

## Accessibility & Inclusion

Comfortable for long dim-light sessions (no harsh pure-white or pure-black extremes, sufficient contrast for small text). Full keyboard operation. Respect Windows high-contrast and reduced-motion settings.
