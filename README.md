# Courland Prospect

A light CRM desktop app for macOS and Windows, built for founder-led outbound sales.
Its design draws from Peter Kazanjy's *Founding Sales*: explicit pipeline stages, fast
lead qualification, and low-friction data hygiene.

Prospects are captured straight from LinkedIn by a bundled Chrome extension, organized
into one pipeline, and replied to with drafts composed by your own local
[Claude Code](https://claude.com/claude-code) install — no API keys, no cloud sync.
Everything lives in a single local SQLite database on your machine.

The model is one product, several kinds of buyer. You describe what you sell **once**;
each **customer profile** says who a kind of buyer is, what they care about, and what a
thread with them should achieve. A draft is then composed by picking, from your single
snippet library, the lines that speak to *that* buyer and move the conversation one step
toward *their* goal.

## What it does

- **Product** — the one thing you sell, written out once. The single source of product
  truth every draft and comment is composed against.
- **Customers** — an ideal-customer profile per kind of buyer: who they are, what they
  care about, and the goal for a thread with them. This is what steers snippet choice;
  it owns no pipeline and no snippets of its own.
- **Prospects & the cycle** — one board everyone moves through, filterable by customer
  profile, with an "awaiting reply" flag derived from captured message history. Each
  prospect points at a customer profile (or none — they still sit in the pipeline, their
  drafts just get no goal).
- **Stage goals & auto-advance** — every stage of the cycle carries a *goal*: what has to
  be true before someone belongs in the next one. Drafts for the people in a stage aim at
  that goal, and after each captured message Claude re-reads the thread and asks whether
  it's been met — surfacing a one-click **suggestion** on the card, with its reasoning.
  Nothing ever moves on its own.
- **Staleness** — each stage sets how many days of silence *from you* turn a card amber,
  then red, so threads you've dropped surface instead of quietly aging.
- **Snippets** — one library of reusable message building blocks, with `[bracket]`
  placeholders filled from context. Each carries two independent tags: a **stage** (*where
  in a conversation* the line fits — the primary axis, which the library groups by and you
  can set by hand) and a **topic** (*what it's about* — AI-derived and read-only). A draft
  prefers lines on the topic the thread is already on, and changes subject when the thread
  gives it a reason to. Drafts compose from your snippets and profile — the substance is
  yours, with blanks filled from the prospect and short connecting sentences added to join
  the lines up.
- **Organize library** — one action re-scores every snippet, re-groups them by stage,
  re-tags what each is about, and flags groups that say the same thing. The redundancy
  report is a review panel: you pick which version to keep, and nothing is deleted until
  you do.
- **Profile** — global "who you are" context: your background, role, and voice.
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
  features/               # one folder per concept — product, customers, prospects,
                          #   stages, profile, snippets, messages, comments, watchlist
                          #   (each: mod / model / repository / commands)
  ai/                     # single path to the local Claude Code CLI (prompt + client + parse)
  ingest/                 # loopback HTTP server, Chrome discovery, heartbeat gate, security
  util.rs                 # shared input bounds

src/                      # React frontend
  app/                    # top-level shell + tabs
  product/ customers/ prospects/ snippets/ profile/  # per-feature views
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
