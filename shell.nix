{ pkgs ? import <nixpkgs> {}, run ? "bash" }:
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
    ]);
    runScript = "${run}";
  }).env
