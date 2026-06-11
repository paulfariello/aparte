# Open architectural findings

These are design tensions surfaced during an architectural review. None are blocking; each needs a dedicated investigation or decision thread before code changes.

## F-01: Winner-takes-all XMPP message dispatch may not hold

`can_handle_xmpp_message` returns a `f64` confidence score; core picks exactly one mod to handle each raw `XmppParsersMessage`. The assumption is that XMPP message types are disjoint. This may break for messages that carry multiple simultaneous payloads (e.g. a correction that also triggers a delivery receipt). If it breaks, the winning mod must re-emit a domain event for the second concern — investigate whether that pattern already appears in practice.

## F-02: Broadcast dispatch has no subscription model

Every `Event` is delivered to every mod. For 15 mods at 60fps, the cost is negligible today. As mods grow, consider a subscription registry where mods declare at `init` time which `Event` variants they handle, and core routes only to subscribers.

## F-03: `UIMode` defined in `core.rs`

`UIMode` is a pure UI concern but lives in `core.rs` because `Event::UIMode(UIMode)` is defined there. Fix: move `UIMode` to `src/mods/ui.rs` and import it in `core.rs` for the event variant. The dependency arrow should point core → ui type, not ui consuming a core-defined UI concept.

## F-04: `read_markers` mod is unregistered dead code

`src/mods/read_markers.rs` is declared in `mod.rs` but has no `Mod::ReadMarkers` variant and is never inserted into the registry. Rule: every mod module must map to a registered mod. Merge into `displayed_markers` or register it.

## F-05: Multi-account has no UX design

`Aparte` supports multiple simultaneous `connections` at the protocol layer, but there is no UX design for switching between or presenting multiple accounts. `current_connection: Option<Account>` is the operative model. Implicit account inference in commands (`:win`, `:msg`, etc.) will become ambiguous if multi-account UX is ever added.

## F-06: `Event::Omemo(OmemoEvent)` inverts the dependency direction

`core.rs` imports `mods::omemo::OmemoEvent` to define its own `Event` enum. This makes core depend on a specific mod's internal type. Tolerated because OMEMO's state machine is complex enough to warrant a sub-event. If other mods follow the same pattern, define a policy (e.g. sub-event types live in a shared `events` module, not in the mod that produces them).

## F-07: Core commands should migrate to owning mods

`:win`, `:close` belong in `ui` mod. `:join`, `:leave`, `:msg` belong in `conversation` mod. `:connect` and `:help` may remain in core. The `:win` completion callback already calls `aparte.get_mod::<UIMod>()` from inside core — a symptom of misplacement. No obstacle to migration beyond inertia.

## F-08: `AparteAsync` split may be collapsible

`AparteAsync` was created when `tokio-xmpp` was `!Send`, making `Aparte` `!Send`. `tokio-xmpp` is now `Send`. Switching `Aparte`'s mod registry from `Rc<RefCell<...>>` to `Arc<Mutex<...>>` would make `Aparte` `Send+Sync` and allow `AparteAsync` to be eliminated. Trade-off: simpler API surface vs. disciplined concurrent mod access.
