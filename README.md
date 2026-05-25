Aparté [![Build Status](https://circleci.com/gh/paulfariello/aparte.svg?style=svg)](https://app.circleci.com/pipelines/github/paulfariello/aparte)
======

A terminal XMPP client written in Rust. Keyboard-driven with a vim-style modal interface, inspired by [Profanity](http://profanity-im.github.io/).

[![asciicast](https://asciinema.org/a/389329.png)](https://asciinema.org/a/389329)

Features
--------

**Messaging**
- Multi-user chat (MUC) and direct messages
- Message Archive Management (MAM) with lazy history fetch
- XEP-0333 displayed markers — track read state in MUC
- XEP-0184 / XEP-0333 delivery status — outgoing 1-on-1 messages show `✓` (delivered) and `✓✓` (read)
- XEP-0444 message reactions — emoji reactions displayed below messages
- XEP-0393 inline message styling — bold, italic, strikethrough
- Inline image display via Sixel

**Privacy**
- OMEMO end-to-end encryption, including MUC
- Decrypted messages are persisted locally so they remain readable after restarts, even though the OMEMO ratchet prevents re-decryption

**Interface**
- Vim-style modal editing — Normal, Insert, and Command modes
- Message search with `/`
- Tab completion for commands, JIDs, and nicks
- Consistent per-contact color generation
- Built-in color themes: `profanity`, `catppuccin-mocha`, `catppuccin-latte`, `catppuccin-frappe`
- Roster and bookmark management

Getting started
---------------

Aparté starts in **Insert mode** — type and press Enter to send. Press `Escape` to enter **Normal mode** for navigation, or `:` to enter **Command mode**.

Connect to your account:

```
:connect me@example.org
```

Join a room:

```
:join room@conference.example.org
```

Switch windows with `Alt+[1-9]` or `:win <name>`.

### Modal interface

| From    | Key        | To / Action                       |
|---------|------------|-----------------------------------|
| Insert  | `Escape`   | Normal mode                       |
| Normal  | `i`        | Insert mode                       |
| Normal  | `:`        | Command mode                      |
| Normal  | `/`        | Search mode                       |
| Normal  | `j` / `↓`  | Select next message               |
| Normal  | `k` / `↑`  | Select previous message           |
| Normal  | `gg`       | Scroll to top                     |
| Normal  | `G`        | Scroll to bottom                  |
| Normal  | `n` / `N`  | Next / previous search match      |
| Command | `Escape`   | Normal mode                       |
| Insert  | `Tab`      | Auto-complete                     |
| Any     | `Ctrl+L`   | Force full screen repaint         |

Commands accept unique prefixes: `:conn` resolves to `:connect` when unambiguous.

### Vim motions in the input bar

Normal mode supports vim-style text motions on the input bar. A motion can be
prefixed with a count (`3w`) and an operator (`d`, `c`, `y`). Text deleted or
yanked is stored in a register and can be pasted with `p` / `P`.

| Key       | Motion                          |
|-----------|---------------------------------|
| `h` / `l` | Left / right one character      |
| `w` / `W` | Forward to next word / WORD     |
| `b` / `B` | Backward to start of word / WORD|
| `e` / `E` | Forward to end of word / WORD   |
| `0`       | Start of line                   |
| `^`       | First non-blank character       |
| `$`       | End of line                     |

| Key       | Operator                        |
|-----------|---------------------------------|
| `d{mot}`  | Delete over motion              |
| `c{mot}`  | Change (delete + enter Insert)  |
| `y{mot}`  | Yank (copy) over motion         |
| `dd`      | Delete whole input              |
| `cc`      | Clear input and enter Insert    |
| `yy`      | Yank whole input                |
| `x` / `X` | Delete char under / before cursor |
| `p` / `P` | Paste after / before cursor     |

Registers: prefix any operator with `"a` (e.g. `"adw`) to use a named register.
Uppercase letters append to the named register (`"Adw`).

### Correcting a sent message

In Normal mode, navigate with `k`/`j` to one of your own sent messages and
press `i`. The cursor moves onto the message itself (steady bar), pre-filled
with the current body. Edit the text and press `Enter` to send the correction
(XEP-0308): the message updates in place and is marked with a `✎` icon. Press
`Esc` to cancel the edit and revert. Incoming messages are not editable.

Once a message is selected, vim text motions (`h`, `l`, `w`, `b`, `e`, `0`,
`$`, count prefixes, etc.) apply to the message body immediately — no extra
keypress needed. Press `i` to enter Insert mode on the message (cursor becomes
a bar, free typing). Press `Esc` to return to Normal mode on the message
(motions apply again). Press `Esc` once more to cancel the edit.

Install
-------

### Cargo

```sh
cargo install aparte
```

### Arch Linux (AUR)

```sh
paru -S aparte-git
# or manually:
git clone https://aur.archlinux.org/aparte-git.git && cd aparte-git && makepkg -si
```

### Guix

```sh
# From the GuixRUs channel (https://git.sr.ht/~whereiseveryone/guixrus):
guix pull && guix install aparte

# Or build from source:
git clone https://github.com/paulfariello/aparte --branch develop
cd aparte && guix package -f guix.scm
```

### Windows (WSL)

Inside a Debian-based WSL environment:

```sh
sudo apt update && sudo apt install libssl-dev pkg-config curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
cargo install --git https://github.com/paulfariello/aparte --branch develop
```

Configuration
-------------

Config file location: `$XDG_CONFIG_HOME/aparte/config.toml` (falls back to `~/.config/aparte/config.toml`).

```toml
# Audio bell on mention
bell = true

# Built-in themes: profanity, catppuccin-mocha, catppuccin-latte, catppuccin-frappe
theme_name = "catppuccin-mocha"

[accounts.example]
jid = "me@example.org/aparte"
autoconnect = true
```

Contact
-------

Join [aparte@conference.fariello.eu](xmpp:aparte@conference.fariello.eu?join)
