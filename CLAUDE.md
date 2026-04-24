# Aparte

## Build & Development

All `cargo` commands must be run inside the Nix dev shell using `./dev`. Prefix every cargo invocation with `./dev`:

```sh
./dev cargo build
./dev cargo test
./dev cargo clippy
# etc.
```

`./dev` is a local script that wraps `nix develop` (defined in `flake.nix`).

## Development Shell (Claude / interactive use)

Claude Code and interactive development sessions use the `dev` shell, which adds neovim, claude-code, git, and pre-commit. Use the `-d` flag:

```sh
# Enter an interactive dev shell
./dev -d
```

Once inside the dev shell, **do not prefix commands with `./dev`** — `cargo`, `pre-commit`, etc. are directly available.

You can also enter the shells directly with Nix:

```sh
nix develop          # default shell (build only)
nix develop .#dev    # dev shell (adds editor/tooling)
```

## Pre-commit Hooks

`.pre-commit-config.yaml` defines the following hooks that run on every commit:

- `fmt` — rustfmt formatting check
- `cargo-check` — compile check with `-D warnings` (base and `--features image`)
- `clippy` — lint check (base and `--features image`)

Install the hooks once after cloning (from inside the dev shell):

```sh
pre-commit install
```

Run manually against all files:

```sh
pre-commit run --all-files
```

## Tests

**Tests MUST be run before each commit** (from inside the dev shell or with `./dev`):

```sh
cargo test
```
