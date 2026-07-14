# Courland Prospect

A light CRM desktop app for macOS and Windows, built for founder-led outbound sales.
Its design draws from Peter Kazanjy's *Founding Sales*: explicit pipeline stages, fast
lead qualification, and low-friction data hygiene.

Prospects are captured straight from LinkedIn by a bundled Chrome extension, organized
into per-pitch pipelines, and replied to with drafts composed by your own local
[Claude Code](https://claude.com/claude-code) install — no API keys, no cloud sync.
Everything lives in a single local SQLite database on your machine.

## What it does

- **Pitches** — each pitch is a self-contained campaign: a name, a "skill" (what you're
  selling), and its own set of pipeline stages. The active pitch scopes the whole app.
- **Prospects & pipeline** — a board of prospects per pitch, movable across stages, with
  an "awaiting reply" flag derived from captured message history.
- **Profile** — global "who you are" / "what you're building" context, reused across
  every pitch's drafts.
- **Snippets** — reusable message building blocks (global or pitch-scoped) with
  `[bracket]` placeholders filled from context. Drafts compose *only* from your snippets
  and profile — nothing is invented.
- **LinkedIn capture** — a Chrome extension adds a capture button to LinkedIn: save a
  person as a prospect and pull their chat history into the CRM.
- **AI drafting & polish** — reply drafts are pre-generated in one inbox pass and opened
  in pre-filled review tabs; free-text fields have a "polish" action. All of it runs
  through your local `claude` CLI in headless mode.
- **Extension gate** — the app stays locked behind a gate screen until the extension
  checks in (a heartbeat over the loopback server), so capture is always wired up when
  you're working.

## Tech stack

- **Backend** — [Tauri v2](https://tauri.app/) (Rust), [rusqlite](https://docs.rs/rusqlite)
  (bundled SQLite), [axum](https://docs.rs/axum) on Tauri's tokio runtime for the loopback
  ingest server.
- **Frontend** — React 19 + TypeScript, built with [Vite](https://vite.dev/), CSS Modules.
- **Extension** — TypeScript content scripts bundled with Vite + [@crxjs](https://crxjs.dev/).
- **AI** — the user's local Claude Code CLI (`claude -p`), reusing its install and auth.

## Architecture

The backend is **feature-first**. Each concept under `src-tauri/src/features/` is a
self-contained vertical slice — its data shape (`model`), persistence (`repository`, where
*all* SQL lives), and exposed API (`commands`). Shared infrastructure that isn't a feature
sits alongside:

```
src-tauri/src/
  lib.rs                  # Tauri builder: opens the DB, manages state, registers commands
  database/               # open() + AppState (single Mutex<Connection>); versioned migrations
  features/               # one folder per concept — pitches, prospects, stages, profile,
                          #   snippets, messages (each: mod / model / repository / commands)
  ai/                     # single path to the local Claude Code CLI (prompt + client)
  ingest/                 # loopback HTTP server, Chrome discovery, heartbeat gate, security
  util.rs                 # shared input bounds

src/                      # React frontend
  app/                    # top-level shell, tabs, active-pitch state
  pitches/ prospects/ profile/  # per-feature views
  gate/                   # gate screen + heartbeat polling
  api/                    # typed wrappers over Tauri commands
  components/ lib/ styles/       # shared UI, hooks, global CSS

chrome-extension/         # LinkedIn capture + draft extension (own Vite build)
```

Adding a feature means a new `features/<feature>/` folder, one line in `features/mod.rs`,
and registering its commands in `lib.rs`. Adding a table means a new
`database/migrations/NNNN_name.sql` file — shipped migrations are never edited, only
appended. See [CLAUDE.md](CLAUDE.md) for the full working conventions.

### Data flow

The Chrome extension talks to the desktop app over a **loopback HTTP server** (bound to
`127.0.0.1`), not the user's default browser automation — it uses your *own* Chrome, no
CDP. Every request is guarded by an exact `Host` check (anti DNS-rebinding), a shared
per-launch token, and a CORS allowlist pinned to the extension's origin. The extension's
periodic `GET /health` doubles as the heartbeat that unlocks the gate.

## Prerequisites

- [Node.js](https://nodejs.org/) 18+ and npm
- [Rust](https://www.rust-lang.org/tools/install) (stable) + the
  [Tauri v2 system dependencies](https://tauri.app/start/prerequisites/) for your platform
- [Claude Code](https://claude.com/claude-code) installed and authenticated, for the AI
  drafting and polish features (the app runs without it — those features are disabled and
  explained in the UI when the `claude` CLI can't be reached)
- Google Chrome, for LinkedIn capture

## Getting started

Install frontend dependencies:

```bash
npm install
```

Run the app in development (starts Vite + the Tauri shell):

```bash
npm run tauri dev
```

Build a production bundle for your platform. This also builds the Chrome extension into
`chrome-extension/dist/` and bundles it as an app resource:

```bash
npm run tauri build
```

### Loading the Chrome extension

The extension is shipped inside the app to a writable directory on first launch, with its
loopback port and shared token provisioned into `config.json`. To load it during
development, build it and load the unpacked `chrome-extension/dist/` folder via
`chrome://extensions` (Developer mode → Load unpacked). Its ID is pinned via a public key
in the manifest so the server's CORS allowlist can name its origin ahead of time.

## Configuration

A few environment variables override discovery when the defaults don't fit:

- `COURLAND_CLAUDE_PATH` — explicit path to the `claude` binary (otherwise common install
  locations and `PATH` are searched).
- `COURLAND_CHROME_PATH` — explicit path to the Chrome executable.

The SQLite database is created and migrated automatically on first launch, in the
platform's per-user app-data directory (`courland-prospect.db`). A failed migration fails
loud and never discards existing data.

## Project scripts

Run from the repo root:

- `npm run tauri dev` — run the desktop app in development
- `npm run tauri build` — build a distributable bundle (also builds the extension)
- `npm run dev` — Vite frontend only (no Tauri shell)
- `npm run build` — type-check and build the frontend

Rust tests (repositories are unit-tested against in-memory SQLite):

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```
