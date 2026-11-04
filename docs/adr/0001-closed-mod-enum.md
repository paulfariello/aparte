# ADR-0001: Closed `Mod` enum instead of `Box<dyn ModTrait>`

**Status:** Accepted

## Context

Mods are the primary extension point. Each mod's `on_event` receives `&mut Aparte`, which owns the mod registry. A `Vec<Box<dyn ModTrait>>` would create a self-referential borrow: to call a mod you must borrow the vec mutably, but you also need to pass `&mut Aparte` (which contains the vec) into the call. The borrow checker rejects this.

A secondary concern was avoiding vtable overhead on the hot dispatch path (every event visits every mod).

## Decision

Mods are variants of a closed `Mod` enum. Dispatch uses exhaustive `match` arms. The inner type is extracted from the enum variant, then called with `&mut Aparte`. No self-referential borrow occurs because the match arm holds a reference to the inner type, not to the enclosing enum.

## Consequences

- Adding a mod requires touching ~5 match arms in `core.rs`: `on_event`, `can_handle_xmpp_message`, `handle_xmpp_message`, `init`, `Display`/`Debug`.
- Zero dynamic dispatch on the event path.
- The mod set is fixed at compile time — no runtime plugin loading.
- "Primary extension point" means "the intended place to add features", not "open for external extension".

## Future

If the borrow checker constraint is ever lifted (e.g. by restructuring `Aparte` ownership), `Box<dyn ModTrait>` or an enum-dispatch crate could open the mod set without the match-arm boilerplate.
