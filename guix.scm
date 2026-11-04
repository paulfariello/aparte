; This is a recipe to setup aparte from the local sources
; To install, simply run guix package -f guix.scm
; And don't forget to update dependencies in here when they're bumped by cargo

(define-module (gnu packages aparte)
  #:use-module (gnu packages crates-io)
  #:use-module (gnu packages tls)
  #:use-module (gnu packages pkg-config)
  #:use-module (guix packages)
  #:use-module (guix download)
  #:use-module (guix git-download)
  #:use-module (guix build-system cargo)
  #:use-module (guix build utils)
  #:use-module  (ice-9 popen)
  #:use-module (ice-9 rdelim)
  #:use-module (guix gexp)
  #:use-module ((guix licenses) #:prefix license:))

(define-public rust-mockall-derive-0.9
  (package
    (name "rust-mockall-derive")
    (version "0.9.1")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "mockall_derive" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "1snywdscj3chgs0xqr5700dsw2hy0qwd0s3kdk4nz85w6m327m2x"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-cfg-if" ,rust-cfg-if-1)
         ("rust-proc-macro2" ,rust-proc-macro2-1)
         ("rust-quote" ,rust-quote-1)
         ("rust-syn" ,rust-syn-1))))
    (home-page "https://github.com/asomers/mockall")
    (synopsis "Procedural macros for Mockall
")
    (description "Procedural macros for Mockall
")
    (license (list license:expat license:asl2.0))))

(define-public rust-fragile-1
  (package
    (name "rust-fragile")
    (version "1.0.0")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "fragile" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "1wlihmkjyhvl5rckal32p010piy1l15s6l81h7z31jcd971kk839"))))
    (build-system cargo-build-system)
    (arguments `(#:skip-build? #t))
    (home-page
      "https://github.com/mitsuhiko/rust-fragile")
    (synopsis
      "Provides wrapper types for sending non-send values to other threads.")
    (description
      "This package provides wrapper types for sending non-send values to other threads.")
    (license license:asl2.0)))

(define-public rust-downcast-0.10
  (package
    (name "rust-downcast")
    (version "0.10.0")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "downcast" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "07bh0l95gwrzak6rj29v8kkm577d8vivxsxhqgscf64b4bq59d2b"))))
    (build-system cargo-build-system)
    (arguments `(#:skip-build? #t))
    (home-page
      "https://github.com/fkoep/downcast-rs")
    (synopsis
      "Trait for downcasting trait objects back to their original types.")
    (description
      "Trait for downcasting trait objects back to their original types.")
    (license license:expat)))

(define-public rust-mockall-0.9
  (package
    (name "rust-mockall")
    (version "0.9.1")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "mockall" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "1m9kpv4523503v48ahyzk9g2rabvbjl70mlbkc8mkfzr4fni9mhq"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-cfg-if" ,rust-cfg-if-1)
         ("rust-downcast" ,rust-downcast-0.10)
         ("rust-fragile" ,rust-fragile-1)
         ("rust-lazy-static" ,rust-lazy-static-1)
         ("rust-mockall-derive" ,rust-mockall-derive-0.9)
         ("rust-predicates" ,rust-predicates-1)
         ("rust-predicates-tree" ,rust-predicates-tree-1))))
    (home-page "https://github.com/asomers/mockall")
    (synopsis
      "A powerful mock object library for Rust.
")
    (description
      "This package provides a powerful mock object library for Rust.
")
    (license (list license:expat license:asl2.0))))

(define-public rust-sha3-0.9
  (package
    (name "rust-sha3")
    (version "0.9.1")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "sha3" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "02d85wpvz75a0n7r2da15ikqjwzamhii11qy9gqf6pafgm0rj4gq"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-block-buffer" ,rust-block-buffer-0.9)
         ("rust-digest" ,rust-digest-0.9)
         ("rust-keccak" ,rust-keccak-0.1)
         ("rust-opaque-debug" ,rust-opaque-debug-0.3))))
    (home-page
      "https://github.com/RustCrypto/hashes")
    (synopsis "SHA-3 (Keccak) hash function")
    (description "SHA-3 (Keccak) hash function")
    (license (list license:expat license:asl2.0))))

(define-public rust-minidom-0.13
  (package
    (name "rust-minidom")
    (version "0.13.0")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "minidom" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "13k0ngkwgj0zgn0zkkklnj274q351mpyzjaglr0dviwz2k19499k"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-quick-xml" ,rust-quick-xml-0.20))))
    (home-page "https://gitlab.com/xmpp-rs/xmpp-rs")
    (synopsis
      "A small, simple DOM implementation on top of quick-xml")
    (description
      "This package provides a small, simple DOM implementation on top of quick-xml")
    (license license:mpl2.0)))

(define-public rust-jid-0.9
  (package
    (name "rust-jid")
    (version "0.9.2")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "jid" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "1s3dl38wwhnx0pbzm4cnwqmmr09nfg4nv6w4yl3cmbkc2n7xipma"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-minidom" ,rust-minidom-0.13)
         ("rust-serde" ,rust-serde-1))))
    (home-page "https://gitlab.com/xmpp-rs/xmpp-rs")
    (synopsis
      "A crate which provides a Jid struct for Jabber IDs.")
    (description
      "This package provides a crate which provides a Jid struct for Jabber IDs.")
    (license license:mpl2.0)))

(define-public rust-blake2-0.9
  (package
    (name "rust-blake2")
    (version "0.9.1")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "blake2" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "1x3qz692hfrxgw6cd94iiid6iqal2dwj6zv5137swpgg4l17598h"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-crypto-mac" ,rust-crypto-mac-0.8)
         ("rust-digest" ,rust-digest-0.9)
         ("rust-opaque-debug" ,rust-opaque-debug-0.3))))
    (home-page
      "https://github.com/RustCrypto/hashes")
    (synopsis "BLAKE2 hash functions")
    (description "BLAKE2 hash functions")
    (license (list license:expat license:asl2.0))))

(define-public rust-xmpp-parsers-0.18
  (package
    (name "rust-xmpp-parsers")
    (version "0.18.1")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "xmpp-parsers" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "064bjmngy0abcp9wcms7h5b13rljr0isliy83csaa0j7xyjmpkq1"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-base64" ,rust-base64-0.13)
         ("rust-blake2" ,rust-blake2-0.9)
         ("rust-chrono" ,rust-chrono-0.4)
         ("rust-digest" ,rust-digest-0.9)
         ("rust-jid" ,rust-jid-0.9)
         ("rust-minidom" ,rust-minidom-0.13)
         ("rust-sha-1" ,rust-sha-1-0.9)
         ("rust-sha2" ,rust-sha2-0.9)
         ("rust-sha3" ,rust-sha3-0.9))))
    (home-page "https://gitlab.com/xmpp-rs/xmpp-rs")
    (synopsis
      "Collection of parsers and serialisers for XMPP extensions")
    (description
      "Collection of parsers and serialisers for XMPP extensions")
    (license license:mpl2.0)))

(define-public rust-pbkdf2-0.6
  (package
    (name "rust-pbkdf2")
    (version "0.6.0")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "pbkdf2" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "0jjaapyawm5iqn97mmfj40dvipsy78cm80qcva28009l2zbw1f5k"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-base64" ,rust-base64-0.13)
         ("rust-crypto-mac" ,rust-crypto-mac-0.10)
         ("rust-hmac" ,rust-hmac-0.10)
         ("rust-rand" ,rust-rand-0.7)
         ("rust-rand-core" ,rust-rand-core-0.5)
         ("rust-rayon" ,rust-rayon-1)
         ("rust-sha2" ,rust-sha2-0.9)
         ("rust-subtle" ,rust-subtle-2))))
    (home-page
      "https://github.com/RustCrypto/password-hashes/tree/master/pbkdf2")
    (synopsis "Generic implementation of PBKDF2")
    (description "Generic implementation of PBKDF2")
    (license (list license:expat license:asl2.0))))

(define-public rust-hmac-0.10
  (package
    (name "rust-hmac")
    (version "0.10.1")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "hmac" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "058yxq54x7xn0gk2vy9bl51r32c9z7qlcl2b80bjh3lk3rmiqi61"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-crypto-mac" ,rust-crypto-mac-0.10)
         ("rust-digest" ,rust-digest-0.9))))
    (home-page "https://github.com/RustCrypto/MACs")
    (synopsis
      "Generic implementation of Hash-based Message Authentication Code (HMAC)")
    (description
      "Generic implementation of Hash-based Message Authentication Code (HMAC)")
    (license (list license:expat license:asl2.0))))

(define-public rust-sasl-0.5
  (package
    (name "rust-sasl")
    (version "0.5.0")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "sasl" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "1j9d6q580r18i90ksr0frjks3mzll73966p2rp0vn9w90b77sbap"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-base64" ,rust-base64-0.13)
         ("rust-getrandom" ,rust-getrandom-0.2)
         ("rust-hmac" ,rust-hmac-0.10)
         ("rust-pbkdf2" ,rust-pbkdf2-0.6)
         ("rust-sha-1" ,rust-sha-1-0.9)
         ("rust-sha2" ,rust-sha2-0.9))))
    (home-page "https://gitlab.com/lumi/sasl-rs")
    (synopsis
      "A crate for SASL authentication. Currently only does the client side.")
    (description
      "This package provides a crate for SASL authentication.  Currently only does the client side.")
    (license license:lgpl3+)))

(define-public rust-tokio-xmpp-3
  (package
    (name "rust-tokio-xmpp")
    (version "3.0.0")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "tokio-xmpp" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "0f8s4gsv9zs6rlkc40jjglcm0prq10ypxszrwvpxhjbygbxrzb2n"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-bytes" ,rust-bytes-1)
         ("rust-futures" ,rust-futures-0.3)
         ("rust-idna" ,rust-idna-0.2)
         ("rust-log" ,rust-log-0.4)
         ("rust-native-tls" ,rust-native-tls-0.2)
         ("rust-rustc-version" ,rust-rustc-version-0.3)
         ("rust-sasl" ,rust-sasl-0.5)
         ("rust-tokio" ,rust-tokio-1)
         ("rust-tokio-native-tls"
          ,rust-tokio-native-tls-0.3)
         ("rust-tokio-stream" ,rust-tokio-stream-0.1)
         ("rust-tokio-util" ,rust-tokio-util-0.6)
         ("rust-trust-dns-proto"
          ,rust-trust-dns-proto-0.20)
         ("rust-trust-dns-resolver"
          ,rust-trust-dns-resolver-0.20)
         ("rust-xml5ever" ,rust-xml5ever-0.16)
         ("rust-xmpp-parsers" ,rust-xmpp-parsers-0.18))))
    (home-page "https://gitlab.com/xmpp-rs/xmpp-rs")
    (synopsis
      "Asynchronous XMPP for Rust with tokio")
    (description
      "Asynchronous XMPP for Rust with tokio")
    (license license:mpl2.0)))

(define-public rust-linked-hash-set-0.1
  (package
    (name "rust-linked-hash-set")
    (version "0.1.4")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "linked_hash_set" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "124m7wiz9ah7ah58ckai413mzfglh3y1nz64qy1s676qlinnq627"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-linked-hash-map"
          ,rust-linked-hash-map-0.5)
         ("rust-serde" ,rust-serde-1))))
    (home-page
      "https://github.com/alexheretic/linked-hash-set")
    (synopsis "HashSet with insertion ordering")
    (description "HashSet with insertion ordering")
    (license license:asl2.0)))

(define-public rust-hsluv-0.1
  (package
    (name "rust-hsluv")
    (version "0.1.0")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "hsluv" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "0g5p4x9np7292fxinqj34vlys5v20hg5yqqr8vvqbw8xcl5l3rax"))))
    (build-system cargo-build-system)
    (arguments `(#:skip-build? #t))
    (home-page
      "https://github.com/bb010g/rust-hsluv.git")
    (synopsis
      "Human-friendly HSL, Rust implementation (revision 4)")
    (description
      "Human-friendly HSL, Rust implementation (revision 4)")
    (license license:expat)))

(define-public rust-flexi-logger-0.15
  (package
    (name "rust-flexi-logger")
    (version "0.15.12")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "flexi_logger" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "1gs2flpzjd4kr9jw614vaqxxz7fd56gqkr78j47q0ja1vfp3raxa"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-atty" ,rust-atty-0.2)
         ("rust-chrono" ,rust-chrono-0.4)
         ("rust-flate2" ,rust-flate2-1)
         ("rust-glob" ,rust-glob-0.3)
         ("rust-hostname" ,rust-hostname-0.3)
         ("rust-lazy-static" ,rust-lazy-static-1)
         ("rust-libc" ,rust-libc-0.2)
         ("rust-log" ,rust-log-0.4)
         ("rust-notify" ,rust-notify-4)
         ("rust-regex" ,rust-regex-1)
         ("rust-serde" ,rust-serde-1)
         ("rust-serde-derive" ,rust-serde-derive-1)
         ("rust-thiserror" ,rust-thiserror-1)
         ("rust-toml" ,rust-toml-0.5)
         ("rust-yansi" ,rust-yansi-0.5))))
    (home-page
      "https://crates.io/crates/flexi_logger")
    (synopsis
      "An easy-to-configure and flexible logger that writes logs to stderr and/or to files. It allows custom logline formats, and it allows changing the log specification at runtime. It also allows defining additional log streams, e.g. for alert or security messages.")
    (description
      "An easy-to-configure and flexible logger that writes logs to stderr and/or to files.  It allows custom logline formats, and it allows changing the log specification at runtime.  It also allows defining additional log streams, e.g.  for alert or security messages.")
    (license (list license:expat license:asl2.0))))

(define-public rust-case-0.1
  (package
    (name "rust-case")
    (version "0.1.0")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "case" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "1hgc6fdg01qfh0qx5c50n717vh0xqvrlvxix8ksng5p291mid2z8"))))
    (build-system cargo-build-system)
    (arguments `(#:skip-build? #t))
    (home-page
      "https://github.com/SkylerLipthay/case")
    (synopsis "A set of letter case string helpers")
    (description
      "This package provides a set of letter case string helpers")
    (license license:expat)))

(define-public rust-derive-error-0.0.4
  (package
    (name "rust-derive-error")
    (version "0.0.4")
    (source
      (origin
        (method url-fetch)
        (uri (crate-uri "derive-error" version))
        (file-name
          (string-append name "-" version ".tar.gz"))
        (sha256
          (base32
            "1j624ma4jw911yg3qqlvfybgk7614k2blhg6wgnb38wyn90882gc"))))
    (build-system cargo-build-system)
    (arguments
      `(#:skip-build?
        #t
        #:cargo-inputs
        (("rust-case" ,rust-case-0.1)
         ("rust-quote" ,rust-quote-0.3)
         ("rust-syn" ,rust-syn-0.11))))
    (home-page
      "https://github.com/rushmorem/derive-error")
    (synopsis
      "Derive macro for Error using macros 1.1")
    (description
      "Derive macro for Error using macros 1.1")
    (license (list license:expat license:asl2.0))))

; Some tricks copied over from 
; https://github.com/ZelenyeShtany/.emacs.d/blob/910ba44ad7198cb0e634fb981484b1ff633de73b/emacs-redshift/guix.scm
; GPL v3 license
(define %source-dir (dirname (current-filename)))

(define (git-output . args)
  "Execute 'git ARGS ...' command and return its output without trailing
newspace."
  (with-directory-excursion %source-dir
    (let* ((port   (apply open-pipe* OPEN_READ "git" args))
           (output (read-string port)))
      (close-port port)
      (string-trim-right output #\newline))))

(define (current-commit)
  (git-output "log" "-n" "1" "--pretty=format:%H"))

(define-public aparte
  (package
    (name "aparte")
    ; TODO: Here we could extract base version (here 0.2.0) from the latest tag
    (version (string-append "0.2.0" "-" (string-take (current-commit) 7)))
    (source (local-file %source-dir
      #:recursive? #t
      #:select? (git-predicate %source-dir)))
    (build-system cargo-build-system)
    (arguments
      `(#:cargo-inputs
        (("rust-backtrace" ,rust-backtrace-0.3)
         ("rust-bytes" ,rust-bytes-0.5)
         ("rust-chrono" ,rust-chrono-0.4)
         ("rust-derive-error" ,rust-derive-error-0.0.4)
         ("rust-dirs" ,rust-dirs-2)
         ("rust-flexi-logger" ,rust-flexi-logger-0.15)
         ("rust-futures" ,rust-futures-0.3)
         ("rust-fuzzy-matcher" ,rust-fuzzy-matcher-0.3)
         ("rust-hsluv" ,rust-hsluv-0.1)
         ("rust-linked-hash-map"
          ,rust-linked-hash-map-0.5)
         ("rust-linked-hash-set"
          ,rust-linked-hash-set-0.1)
         ("rust-log" ,rust-log-0.4)
         ("rust-rand" ,rust-rand-0.8)
         ("rust-rpassword" ,rust-rpassword-3)
         ("rust-rust-crypto" ,rust-rust-crypto-0.2)
         ("rust-serde" ,rust-serde-1)
         ("rust-termion" ,rust-termion-1)
         ("rust-textwrap" ,rust-textwrap-0.12)
         ("rust-tokio" ,rust-tokio-1)
         ("rust-tokio-xmpp" ,rust-tokio-xmpp-3)
         ("rust-toml" ,rust-toml-0.5)
         ("rust-unicode-segmentation"
          ,rust-unicode-segmentation-1)
         ("rust-uuid" ,rust-uuid-0.7)
         ("rust-xmpp-parsers" ,rust-xmpp-parsers-0.18))
        #:cargo-development-inputs
        (("rust-mockall" ,rust-mockall-0.9))))
    (native-inputs
     `(("pkg-config" ,pkg-config)))
    (inputs
     `(("openssl" ,openssl)))
    (home-page
      "https://github.com/paulfariello/aparte")
    (synopsis
      "Simple XMPP console client written in Rust and inspired by Profanity.")
    (description
      "Simple XMPP console client written in Rust and inspired by Profanity.")
    (license license:mpl2.0)))

aparte
