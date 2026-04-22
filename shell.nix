{ pkgs ? import <nixpkgs> {}, run ? "bash", extraPkgs ? (_: []) }:
  (pkgs.buildFHSEnv {
    name = "aparte-env";
    targetPkgs = pkgs: (with pkgs; [
      rustc
      cargo
      rust-analyzer
      rustfmt

      gcc
      pkg-config
      openssl
      sqlite
      cacert
    ]) ++ (extraPkgs pkgs);
    runScript = "${run}";
    profile = ''
      export SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt
      export NIX_SSL_CERT_FILE=/etc/ssl/certs/ca-bundle.crt
      export CARGO_HTTP_CAINFO=/etc/ssl/certs/ca-bundle.crt
    '';
  }).env
