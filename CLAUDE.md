# Aparte

## Build & Development

All `cargo` commands must be run inside the Nix shell using `./fhs`. Prefix every cargo invocation with `./fhs`:

```sh
./fhs cargo build
./fhs cargo test
./fhs cargo clippy
# etc.
```

`./fhs` is a local script that drops you into the FHS-compatible environment defined in `shell.nix`.
