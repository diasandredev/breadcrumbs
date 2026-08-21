# Breadcrumbs

A local-first dev diary for macOS. Sits in your menu bar, watches your
[OpenCode](https://opencode.ai) sessions and your git history, and turns them
into ready-to-paste daily standups, weekly and monthly recaps — with optional
AI polish via a fully local [Ollama](https://ollama.com) model.

## How it works

- **Capture** — reads OpenCode's local database (`~/.local/share/opencode/opencode.db`)
  read-only. No plugins or hooks needed; past sessions are imported on first run.
- **Git** — recent commits for every detected project are woven into the same
  timeline as AI sessions (matched by time window).
- **Storage** — everything lands in one SQLite file at
  `~/Library/Application Support/com.andredias.breadcrumbs/diary.db`.
  Copy that folder to back up or move machines.
- **Privacy** — all data stays on your machine. The only network call ever made
  is to Ollama on localhost, and only if you enable it.

## Features

- Menu bar panel with day-grouped timeline: sessions + commits per project
- Per-session metrics: model, agent, tokens, messages, duration, +/- lines, cost
- Deterministic markdown reports (Today / This week / Last 7 days / This month)
- One-click "Copy today's report" from the tray menu
- Optional AI summaries via local Ollama (`✦ Enhance with AI`)

## Development

```bash
bun install
bun run tauri dev      # run the app
bun run tauri build    # produce .app/.dmg
cargo test             # backend tests (src-tauri)
```

Requirements: Rust toolchain (`rustup`), Bun, Xcode Command Line Tools,
OpenCode and optionally Ollama running locally.

## Roadmap ideas

- More session sources (Claude Code hooks, Cursor, Codex…) behind the same sync trait
- Animated share cards
- Export report as file (save dialog)
