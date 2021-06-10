Aparté [![Build Status](https://www.travis-ci.com/paulfariello/aparte.svg?branch=master)](https://www.travis-ci.com/paulfariello/aparte)
======

Simple XMPP console client written in Rust and inspired by [Profanity](http://profanity-im.github.io/).

Demo
====

[![asciicast](https://asciinema.org/a/389329.png)](https://asciinema.org/a/389329)

Features
========

  - [x] Channel
  - [x] Roster
  - [x] Auto completion
  - [x] Bookmarks
  - [x] Consistent color generation
  - [x] MAM
  - [ ] Omemo

Install
=======

From sources
------------

```
cargo install --git https://github.com/paulfariello/aparte --branch develop
```

Package for Archlinux
---------------------

AUR package is available: `aparte-git`.

```
git clone https://aur.archlinux.org/aparte-git.git
cd aparte-git
makepkg -si
```

Or with your favorite aur-helper:

```
paru aparte-git
```

Windows with WSL
----------------

Aparté should be available inside the Windows subsystem for Linux.
The following instruction are made for a Debian based subsystem (debian or ubuntu for example).

First enter the WSL:

```
PS C:\> debian
```

Then ensure the required dependencies are installed.

```
sudo apt update
sudo apt install libssl-dev pkg-config curl
```

Rust can be installed with rustup.

```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
```

Finally install Aparté.

```
cargo install --git https://github.com/paulfariello/aparte --branch develop
```

Configuration
=============

Aparté can be configured with a configuration file.
The configuration file should be placed in
`$XDG_CONFIG_HOME/aparte/config.toml`. If `$XDG_CONFIG_HOME` is not set,
Aparte will fallback to `$HOME/.config/aparte/config.toml`.

The configuration file should look like the following:

```
[accounts]

[accounts.example]
jid = "me@example.org/aparte"
autoconnect = true
```

Contact
-------

Join [aparte@conference.fariello.eu](xmpp:aparte@conference.fariello.eu?join)
