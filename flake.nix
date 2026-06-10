{
  description = "Aparte — XMPP console client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixpkgs-local.url = "path:/home/needle/workspace/nixpkgs/";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, nixpkgs-local, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
          config.allowUnfree = true;
        };

        pkgs-local = import nixpkgs-local {
          inherit system;
        };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        commonPackages = with pkgs; [
          rustToolchain
          rust-analyzer
          gcc
          pkg-config
          openssl
          openssl.dev
          sqlite
          cacert
          protobuf
        ];

        sslHook = ''
          export SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
          export NIX_SSL_CERT_FILE="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
          export CARGO_HTTP_CAINFO="${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
          export OPENSSL_NO_VENDOR=1
          export PROTOC="${pkgs.protobuf}/bin/protoc"
          export PATH="${pkgs.rust-analyzer}/bin:$PATH"
        '';

      in {
        devShells = {
          default = pkgs.mkShell {
            name = "aparte";
            packages = commonPackages;
            shellHook = sslHook;
          };

          dev = pkgs.mkShell {
            name = "aparte-dev";
            packages = commonPackages ++ (with pkgs; [
              neovim
              claude-code
              git
              pre-commit
              asciinema
            ]) ++ (with pkgs-local; [
              python3
              python3Packages.mempalace
            ]);
            shellHook = sslHook;
          };
        };
      }
    );
}
