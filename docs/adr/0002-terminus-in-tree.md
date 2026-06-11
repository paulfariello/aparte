# ADR-0002: Terminus kept in-tree as a workspace crate

**Status:** Accepted (provisional)

## Context

Aparte needs a custom TUI library: the terminal layout model, sixel image rendering, wide-character handling, and the modal cursor model do not fit existing crates closely enough to avoid significant adaptation. This library is called `terminus`.

Splitting `terminus` into a separate published crate from the start would impose a versioning ceremony (crates.io publish, semver bumps, separate release cycle) on every change to its API, and the API is still changing frequently as aparte's UI evolves.

## Decision

`terminus` lives as a Cargo workspace member (`terminus/`) inside the aparte repository. It shares the same branch, CI, and release cadence as aparte. It is not published to crates.io.

## Consequences

- `terminus` API can be changed freely without a version bump dance.
- `terminus` cannot be used by other projects without vendoring.
- No public API surface obligation — terminus is shaped by aparte's needs alone.

## Future

Extract `terminus` as a standalone published crate once its API stabilises. At that point the inward-facing constraint (aparte only) becomes a public contract, and the API should be reviewed for generality before extraction.
