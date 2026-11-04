# ADR-0003: Single-threaded synchronous event loop

**Status:** Accepted (under review)

## Context

The event loop dispatches every `Event` to every registered mod synchronously on one thread. `Aparte` uses `Rc<HashMap<TypeId, RefCell<Mod>>>`, making it `!Send`. Async work (IQ round-trips, MAM fetch, OMEMO key exchange) escapes via `AparteAsync`, a `Send`-able proxy that communicates back to the loop through an `mpsc` channel.

The original reason `Aparte` was `!Send` was that `tokio-xmpp` was itself `!Send`. `AparteAsync` was created to give spawned tasks a handle back to the event system without requiring `Aparte` to cross thread boundaries.

## Decision

Keep the event loop on a single thread. Use `Rc`/`RefCell` (not `Arc`/`Mutex`) for the mod registry. Async tasks capture `AparteAsync`; they re-enter the sync loop by scheduling events.

## Consequences

- No data races between mods: at most one mod's `on_event` runs at a time.
- `AparteAsync` duplicates `schedule`, `log`, `error`, `current_account` from `Aparte`.
- The split is a maintenance surface: two structs must be kept consistent.

## Future

`tokio-xmpp` is now `Send`. Switching `Aparte` to `Arc`/`Mutex` would make it `Send+Sync`, allowing `AparteAsync` to be eliminated. The trade-off: simpler struct surface vs. needing discipline about concurrent mod access (no inter-mod borrow while another mod holds a lock). Worth revisiting when the mod count stabilises.
