Aparté [![Build Status](https://www.travis-ci.org/paulfariello/aparte.svg?branch=master)](https://www.travis-ci.org/paulfariello/aparte)
======

Simple XMPP console client written in Rust and inspired by Profanity.

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

Contact
-------

Join [aparte@conference.fariello.eu](xmpp:aparte@conference.fariello.eu?join)
