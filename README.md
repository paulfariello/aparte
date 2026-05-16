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
