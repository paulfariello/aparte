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
