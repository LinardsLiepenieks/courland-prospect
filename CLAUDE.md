# Courland Prospect

A light CRM desktop app (macOS + Windows) built on **Tauri v2** (Rust backend) + **React 19 + TypeScript** (Vite). Design philosophy draws from Peter Kazanjy's *Founding Sales* — explicit pipeline stages, fast lead qualification, low-friction data hygiene.

## How to work in this repo

### Design
- When working on anything design-related — UI, components, animation, interaction, visual polish — **use the `emil-design-eng` skill**. Craft and the invisible details matter here.

### Architecture
- **Favor modularity.** Small, focused modules with clear boundaries; prefer composition over large multi-purpose files. A new feature should slot in without forcing edits across unrelated code.
- **Check for existing patterns before writing new code.** When adding something, first search the codebase for a similar pattern, component, or utility already in place — reuse or extend it instead of reinventing. Always look for duplication when building a feature and consolidate rather than copy-paste.

### Research
- **Look things up online yourself when relevant.** If an API, library version, error, or best practice is uncertain, search/fetch to confirm rather than guessing — don't wait to be told.

### Restraint
- **Push back against overengineering.** Prefer the simplest thing that solves the actual problem. Call out speculative abstractions, premature generalization, and unnecessary dependencies before building them — and propose the leaner alternative.

### Subagents & agent teams
- **Choose the right parallelism deliberately.** Use a single subagent for isolated, well-scoped work (a focused search, one contained task). Reach for agent teams / multi-agent orchestration only when the work genuinely decomposes into independent parallel streams or needs adversarial verification — not for linear tasks a single pass handles. When unsure, do it inline; don't spin up agents for their own sake.
