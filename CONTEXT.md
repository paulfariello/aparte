<!-- Aparte domain glossary. Implementation details do not belong here. -->

# Domain glossary

## Account
A connected XMPP identity, represented as a `FullJid` (user@domain/resource). An account is the unit of connection — one TCP session, one credential, one presence. Multiple accounts can be connected simultaneously, though there is no UX design for multi-account yet.

## Conversation
A named communication context. Two subtypes:

- **Chat** — a direct-message exchange between the local account and one remote `BareJid`.
- **Channel** — a multi-user chat room (MUC). Carries a roster of **Occupants** and an optional human-readable name.

A Conversation is identified by `(account, bare_jid)`.

## Occupant
A participant currently present in a Channel. Carries a nick, an optional real JID, an affiliation (Owner / Admin / Member / Outcast / None), and a role (Moderator / Participant / Visitor / None).

## Message
A unit of content displayed in a Conversation window. May originate from XMPP, from the local user, or from the system (log lines). Carries an ID, timestamp, body, sender, and optional encryption/correction/reaction metadata.

## Correction
A user-initiated revision of an outgoing Message, sent via XEP-0308. A Correction replaces the body of the original Message in place. A Correction is either **committed** (sent to the server and applied locally) or **discarded** (abandoned without sending).

## Edit Session
The transient state of a Message being revised locally before the Correction is committed or discarded. An Edit Session is **dirty** when the user has made at least one change to the buffer; it is **clean** when the buffer has not been modified (navigation cursor only). Dirty Edit Sessions are suspended on navigation and survive until explicitly committed or discarded. Clean Edit Sessions are discarded on navigation.

## Command
A user-issued instruction prefixed with `:` (e.g. `:join`, `:win`). Commands are parsed from the input bar and dispatched through the event loop. Each command has a parser, completion callbacks, and a handler function.

## Mod
A self-contained unit of behaviour that handles a subset of Events. Mods implement the `ModTrait` and are registered at startup. They are the primary extension point for new features. Each mod module must correspond to exactly one registered mod.

## Event
The single currency of the event loop. Everything — key presses, XMPP stanzas, render ticks, user commands, internal state transitions — travels as an `Event`. Mods communicate with each other exclusively by scheduling new Events; direct mod-to-mod calls do not exist.

## Terminus
The in-tree TUI layout and rendering library. Provides `View`, `ScreenFrame`, `ScrollWin`, and related primitives. Intended for eventual extraction as a standalone crate once its API stabilises.

## ConnectionInfo
Configuration for connecting an Account: JID, optional server override, optional port, autoconnect flag. Distinct from `Account` (which is the live identity after connection).
