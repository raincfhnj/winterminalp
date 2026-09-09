# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-09-09

Initial public release. WinTerminalP was rewritten from an abandoned Tauri/xterm
terminal into a headless controller that enhances the native Windows Terminal.

### Added

- tmux-style two-stage prefix (`Ctrl+B`, then a second key) active only while Windows
  Terminal is in the foreground.
- Four-direction pane split, focus, and resize, plus new/next/previous/index tab
  activation, close pane, zoom, and native tab rename.
- Mouse divider dragging inferred from disposable UI Automation rectangles and
  translated to native `resizePane` steps.
- TOML configuration for the prefix, timeout, and every action chord, plus mouse resize
  tuning.
- Lossless JSONC integration: an action fragment, hidden bridge keybindings, raw-byte
  backups, SHA-256 compare-and-swap writes, a committed manifest, and reversible
  uninstall that preserves user-modified entries.
- Managed `OSC 9;9` PowerShell profile integration so duplicated panes inherit the
  working directory, with encoding preservation and idempotent, reversible install.
- Per-monitor DPI awareness so hook coordinates and UI Automation rectangles share a
  coordinate system.
- `winter`, `winterminalp`, and hidden `winterd` binaries sharing one CLI.
- `plan`, `install`, `uninstall`, `doctor`, `config`, `run`, and `launch` commands.
- One-time `install.ps1` that builds the release binaries and puts `winter` on `PATH`.
- `winter` and `winter launch` now install the Windows Terminal integration
  automatically on first launch, so a fresh setup no longer needs a separate
  `winter install` step.
- UAC self-relaunch for controller entry points and elevation checks in the core.

### Fixed

- Accept Windows Terminal command-style keybindings that omit an `id` and reserve every
  chord of multi-chord entries instead of treating them as malformed.
- Replay the configured prefix directly so custom prefixes work without reinstalling the
  bridge.
- Pass pointer moves through during divider drags so the hardware cursor keeps reporting
  positions and the drag delta advances.
- Focus the leading pane once per drag instead of on every resize step.

[Unreleased]: https://github.com/raincfhnj/winterminalp/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/raincfhnj/winterminalp/releases/tag/v0.2.0
