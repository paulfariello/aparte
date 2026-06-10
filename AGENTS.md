# Aparte

Aparté is a terminal XMPP client written in Rust with a vim-style modal interface (Insert / Normal / Command modes), inspired by Profanity. It supports MUC, direct messages, MAM, OMEMO encryption, inline Sixel images, and a custom TUI rendering library (`terminus`).

## Architecture

The application is built around a central event loop in `src/core.rs`. An `Event` enum carries everything from XMPP stanzas to key presses; the core dispatches each event to registered mods in order.

**Mods** (`src/mods/`) are the primary extension point. Each mod implements the `Mod` trait and handles the subset of events it cares about. Mods cover distinct concerns:

| Mod | Responsibility |
|---|---|
| `ui` | Terminal rendering, input handling, vim modal state |
| `messages` | Message send/receive, display |
| `reactions` | XEP-0444 message reactions |
| `mam` | Message Archive Management (history fetch) |
| `carbons` | Message carbons (multi-device) |
| `omemo` | End-to-end encryption |
| `disco` | Service discovery |
| `bookmarks` | Bookmark management |
| `completion` | Tab completion for commands, JIDs, nicks |
| `correction` | Last-message correction |
| `displayed_markers` | XEP-0333 read markers |
| `conversation` / `contact` | Conversation and roster state |

**Domain types** live in `src/`: `Message`, `Conversation`, `Channel`, `Account`, `Config`, `Command`.

**Storage** (`src/storage/`) uses Diesel with SQLite. Migrations are in `migrations/`. Schema is generated and checked in.

**Terminus** (`terminus/`) is an in-tree TUI layout and rendering library. It provides `View`, `ScreenFrame`, `ScrollWin`, and related primitives used by the `ui` mod.

## Workflow

### Bug fix or small feature — red-green TDD

1. Write a failing test that reproduces the bug or specifies the new behaviour. Run it to confirm it fails.
2. Write the minimal production code to make the test pass.
3. Refactor if needed, keeping the test green.
4. Run the full test suite and pre-commit before committing (see below).

### Larger feature — plan first

Use `/plan` (or `EnterPlanMode`) to design the approach before touching code. Identify which mods and domain types are affected, how events will flow, and what storage changes (if any) are needed. Get alignment on the plan, then implement following the red-green cycle above.

### Always respect the architecture

- New XMPP features belong in a new or existing **mod**, not in `core.rs`.
- Cross-cutting state goes through `Event` variants, not direct mod-to-mod calls.
- Rendering belongs in the `ui` mod and `terminus`; business logic does not.
- Storage changes require a Diesel migration.

### Document new features

Every new feature must be documented in `README.md`. Update the relevant section or add a new one describing what the feature does and how to use it.

### Before every commit

```sh
cargo test
pre-commit run --all-files
```

Both must pass cleanly. Pre-commit runs `rustfmt`, `cargo check -D warnings`, and `clippy` (with and without `--features image`).

## Build

```sh
cargo build
cargo test
```

Install pre-commit hooks once:

```sh
pre-commit install
```

## Debug

### Screen recording

Aparté can record a session for post-mortem rendering debugging. Pass `--record <path>` on the CLI and it writes raw PTY bytes and reference-screen snapshots to `<path>`:

```sh
cargo run -- --record /tmp/aparte-session.rec [other args]
```

The recording is handled by `src/tee_writer.rs` (`TeeWriter`). `dump_buffer` writes the internal `reference_screen` state to the events file so you can see exactly what the renderer believed the terminal looked like at any point.

To reproduce a rendering glitch deterministically:
1. Run with `--record`, trigger the glitch, quit.
2. Inspect the recorded bytes with a vt100 parser (e.g. `cat /tmp/aparte-session.rec | vt100_player`) or replay them into a test harness.
3. Once you can reproduce the glitch, write a red integration test in `tests/ui_mode.rs` using the PTY harness in `tests/common/mod.rs`.

---

## Ask before

Stop and confirm before any of these. Read-only inspection is fine; state-changing actions are not.

- `git push --force` or `--force-with-lease` to shared branches
- Modifying CI/CD config
- Generating, rotating, or committing any credential, key, or cert
- `rm -rf` outside the current repo working tree
- Adding a new third-party dependency — justify in the commit body

---

## Orchestration

### Subagents to preserve context

The main agent orchestrates and holds the user-facing thread. Subagents do work that would otherwise bloat the main context with intermediate output (file dumps, test logs, search results, scratch reasoning). Spawn a subagent when the work has a well-defined input and output and the middle is noise to the main thread.

Default split for non-trivial changes:

- **Main agent**: planning, decomposition, integrating results, talking to the user
- **Implementer subagent**: writes the code given a clear spec; returns a diff summary, not the full file dumps
- **Reviewer subagent**: reads the diff against the spec and the repo's conventions; returns a list of issues, not a re-narration of the code

The reviewer must be a separate subagent from the implementer. Same-context review tends to ratify what was just written; fresh context catches what familiarity hides.

Parallelize with Task when work is independent:

- Reading independent files across a large repo
- Searching multiple repos for the same pattern
- Running independent test suites
- Gathering context from unrelated subsystems before a plan

Sequential tool calls are fine for dependent work. Don't fan out to look busy.

### `/loop` to convergence

For tasks with an objective pass/fail signal (tests, lint, type-check, plan-clean), use `/loop` to drive cycles until both:

1. The objective signal is green, AND
2. The reviewer subagent returns no blocking issues

Each iteration:

1. **Implementer** makes a change toward the goal
2. **Signal check** runs the objective command (test/lint/plan/etc.)
3. **Reviewer** (fresh context, not the implementer) reads the diff against the spec and the repo's conventions, returns a structured list: blocking issues, suggestions, nits
4. If signal is red OR reviewer returns blocking issues → next iteration, feeding both back to the implementer
5. If signal is green AND reviewer has no blockers → exit

The reviewer is inside the loop, not after it. A green test suite with garbage code still iterates. A clean review with red tests still iterates.

Blocking vs non-blocking is the reviewer's call, but the categories are fixed:

- **Blocking**: correctness, security, violates a rule in this file or the relevant skill, breaks an interface contract, introduces a regression risk the tests don't cover
- **Suggestion**: real improvement, not required to ship
- **Nit**: style preference, ignorable

Only blockers gate the exit. Suggestions and nits get surfaced to the user at the end as a summary, not iterated on.

Cap iterations (default: 5). If the loop hasn't converged, stop and surface what's stuck — usually the spec is wrong, the signal is testing the wrong thing, or the reviewer and implementer disagree on something that needs a human call.

Good `/loop` targets:

- Failing test suite → all green AND review-clean
- Lint or vet errors → zero AND review-clean
- Type errors → clean compile AND review-clean

Bad `/loop` targets: anything where "done" is subjective at the spec level (style, naming, architecture). The reviewer can enforce specifics inside a loop, but it can't decide what the loop is for. Those need a human, not an automated cycle.

---

## Git

- Conventional commits, explaining _why_ not _what_: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`.
- Small, focused, atomic, tested. Each commit must be `git bisect`-safe.
- Always on a dedicated branch.
- Use `git pushdev` instead of plain `git push`.
- Pre-commit hook should reject strings matching common secret patterns (`AKIA*`, `ghp_*`, `eyJ*`, `-----BEGIN`). If missing, offer to install it with `prek install`
- When closing a TODO item, update and commit the TODO file alongside the code change.

---

## Testing

- TDD: write the failing test first.
- Critical flows get unit + integration + E2E.

---

## Comments

Default to no comment. Code should be self-explanatory through naming and structure.

**Only write a comment when it answers _why_, never _what_.** If the comment restates what the code does, delete it and improve the code instead.

**Forbidden:**

- Restating the code in English (`// increment counter` above `counter++`)
- Section banners (`// --- HELPERS ---`)
- Narrating obvious control flow (`// loop through users`)
- TODOs without a ticket reference or owner
- Docstrings that just list parameters already visible in the signature
- Preamble explaining what you're about to do
- Marking what changed in this edit (`// updated to handle null`) — that's what the diff is for

**Acceptable, when truly needed:**

- Non-obvious _why_: business rule, spec reference, workaround for an external bug, performance trade-off, intentional deviation from the obvious approach
- Warnings about non-local consequences ("called by X under Y condition")
- Links to issues, RFCs, or external docs

**Style:** one line is the target. If the explanation needs more lines than the code, refactor instead. Re-read nearby comments when editing; delete or update any that no longer match.

---

## Code style

- No emojis in code, comments, or documentation.
- Documentation under `./docs/` at repo root; nested layout is fine.
- Structured logs everywhere

---

## Privacy

- Never paste secrets in logs, commits, or chat: API keys, tokens, passwords, JWTs, cloud creds, SSH keys.
- Review tool output before quoting it back; redact anything sensitive.
- If a secret was committed, surface it immediately and propose rotation — don't quietly rewrite history.

---

## Communication

- Drop filler: _just_, _really_, _basically_, _actually_, _simply_.
- Drop pleasantries: _sure_, _certainly_, _of course_, _happy to_.
- Prefer short synonyms: _big_ not _extensive_, _fix_ not _implement a solution for_.
- Abbreviate common terms: DB, auth, config, req, res, fn, impl.
- Technical terms stay exact. Code blocks unchanged. Errors quoted exact.
