# Courland Prospect

A light CRM desktop app (macOS + Windows) built on **Tauri v2** (Rust backend) + **React 19 + TypeScript** (Vite). Design philosophy draws from Peter Kazanjy's *Founding Sales* — explicit pipeline stages, fast lead qualification, low-friction data hygiene.

## How to work in this repo

### Design
- When working on anything design-related — UI, components, animation, interaction, visual polish — **use the `emil-design-eng` skill**. Craft and the invisible details matter here.

### Architecture
- **Favor modularity.** Small, focused modules with clear boundaries; prefer composition over large multi-purpose files. A new feature should slot in without forcing edits across unrelated code.
- **Check for existing patterns before writing new code.** When adding something, first search the codebase for a similar pattern, component, or utility already in place — reuse or extend it instead of reinventing. Always look for duplication when building a feature and consolidate rather than copy-paste.

#### Backend structure (Rust / Tauri)

The backend is **feature-first**. `src-tauri/src/`:

```
lib.rs                  # Tauri builder: opens the DB, manages AppState, registers commands
database/               # shared infra — no feature logic
  mod.rs                #   open() + AppState (single Mutex<Connection>)
  migrations/
    mod.rs              #   versioned runner (user_version pragma, per-tx steps)
    NNNN_name.sql       #   one file per migration, embedded via include_str!
features/               # one folder per concept, each a self-contained slice
  mod.rs                #   declares each feature module (`pub mod pitches;` ...)
  <feature>/            #   e.g. pitches/
    mod.rs              #     `pub mod commands;` + private `model`/`repository`
    model.rs            #     struct(s) + `from_row` mapping
    repository.rs       #     ALL SQL; fns take &Connection, no Tauri types (unit-tested in-memory)
    commands.rs         #     #[tauri::command] handlers — the frontend's only entry point
```

A feature is a **vertical slice**, not just a controller: it owns its data shape (`model`), its persistence (`repository`), and its exposed API (`commands`). The controller is just the `commands.rs` file within it.

**Rules for every feature:**
- Adding a feature = one new `features/<feature>/` folder with those four files + one line in `features/mod.rs` + register its commands in `lib.rs`. Don't touch unrelated modules.
- **All SQL lives in `repository.rs`.** Commands lock the connection, validate input, delegate to the repository, and map errors to `String`. Repositories never import Tauri types.
- Register commands by full path: `tauri::generate_handler![features::<feature>::commands::foo]` (a `pub use` of the fn won't expose the macro's hidden items).
- Adding a table = a new `database/migrations/NNNN_name.sql` file + one `include_str!` line. Never edit a shipped migration; only append.
- Migrations are non-destructive by contract — a failed open fails loud, it must never discard the user's DB.

`features/pitches/` is the reference instance — copy its shape.

### Research
- **Look things up online yourself when relevant.** If an API, library version, error, or best practice is uncertain, search/fetch to confirm rather than guessing — don't wait to be told.

### Restraint
- **Push back against overengineering.** Prefer the simplest thing that solves the actual problem. Call out speculative abstractions, premature generalization, and unnecessary dependencies before building them — and propose the leaner alternative.

### Shell commands
- **Avoid `cd` prefixes and compound commands.** The working directory persists across tool calls and already starts at the repo root, so run tools directly (`cargo build`, `npm run …`) instead of `cd … && …`. The Bash permission matcher can't cleanly resolve compound/looped commands (`&&`, `;`, `|`, `for`/`do`) against the allowlist, so they prompt even when each piece is allowed. Target other paths with `--manifest-path` or `git -C <path>` rather than `cd`.
- **Prefer dedicated tools over shell for file work.** Use Read/Glob/Grep to inspect files instead of `cat`/`find`/`grep` in Bash — they skip permission matching entirely. Keep Bash for things that genuinely need a shell, as single flat commands.

### Subagents & agent teams
- **Choose the right parallelism deliberately.** Use a single subagent for isolated, well-scoped work (a focused search, one contained task). Reach for agent teams / multi-agent orchestration only when the work genuinely decomposes into independent parallel streams or needs adversarial verification — not for linear tasks a single pass handles. When unsure, do it inline; don't spin up agents for their own sake.
