{
  description = "Aparte — XMPP console client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
    microvm = {
      url = "github:microvm-nix/microvm.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, microvm, home-manager }:
    let
      # A microvm is a KVM guest, so the runner only exists for Linux hosts; on darwin the
      # only hypervisor microvm.nix offers is vfkit, which supports neither 9p nor virtiofs
      # and so cannot share the working tree in at all.
      vmSystems = [ "x86_64-linux" "aarch64-linux" ];

      # Impure by necessity: a 9p share has to name a host path at evaluation time, and
      # only the caller knows where the checkout -- or, for claudeDir, the per-project
      # .claude-auth to persist -- lives. Empty under a pure evaluation, in which case the
      # guest is built with no share rather than failing.
      srcDir = let e = builtins.getEnv "APARTE_SRC"; in if e != "" then e else null;
      # pathExists, not just non-empty: qemu is handed the share path at evaluation time and
      # refuses to start when it does not resolve, so a host that has never run claude-code
      # would otherwise be unable to boot the guest at all.
      claudeDir =
        let e = builtins.getEnv "APARTE_CLAUDE_DIR";
        in if e != "" && builtins.pathExists e then e else null;

      # The host's own home-manager config, imported rather than copied: nvim, git and the
      # shell then behave in the guest the way they do outside it. Same guard as above, so a
      # machine that manages no home still builds a guest. Everything the config reaches by
      # relative path is copied into the world-readable store unfiltered, secrets included.
      hmDir =
        let e = builtins.getEnv "APARTE_HM_DIR";
        in if e != "" && builtins.pathExists e then e else null;

      # Read rather than shared: `git config --global` is a symlink into the host's
      # /nix/store under home-manager, and that store path does not exist in the guest. The
      # contents land in the world-readable store on both ends, so a config carrying a token
      # in a url.insteadOf or a credential helper does not belong here.
      gitConfig =
        let e = builtins.getEnv "APARTE_GITCONFIG";
        in if e != "" && builtins.pathExists e then builtins.readFile e else null;

      guest = system: nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [
          microvm.nixosModules.microvm
          home-manager.nixosModules.home-manager
          (import ./nix/vm.nix { inherit rust-overlay srcDir claudeDir gitConfig hmDir; })
        ];
      };
    in
    # Exposed so the guest can be inspected without booting it -- `nix eval --impure
    # .#nixosConfigurations.aparte-dev.config.microvm.shares` is how you check the working
    # tree is actually being shared in, which the runner alone cannot tell you.
    {
      nixosConfigurations.aparte-dev = guest "x86_64-linux";
    }
    //
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
          config.allowUnfree = true;
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
        packages = nixpkgs.lib.optionalAttrs (builtins.elem system vmSystems) {
          dev-vm = (guest system).config.microvm.declaredRunner;
        };

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
            ]);

            # Interactive `nix develop .#dev` (and `./dev -d`) boots the microvm and lands
            # you in it. `nix develop .#dev --command ...` — which `./dev -d <cmd>` uses —
            # runs a non-interactive bash, so the test below is false and the command runs
            # on the host against the package set above, exactly as it did before. The two
            # uses want different things and `$-` is the only thing that distinguishes them.
            #
            # Re-entered rather than launched in-process because the shares have to be
            # declared at evaluation time: this evaluation was pure and knows no host path,
            # the nested one is given APARTE_SRC / APARTE_CLAUDE_DIR / APARTE_GITCONFIG /
            # APARTE_HM_DIR and --impure.
            shellHook = sslHook + ''
              if [[ $- == *i* ]]; then
                export APARTE_SRC="''${APARTE_SRC:-$PWD}"
                # A project-local dir rather than the shared $HOME/.claude: that default let
                # tuprfr and aparte's guests share one host credentials.json, and two claude
                # processes racing the same OAuth refresh token forced spurious re-auth
                # whenever both VMs ran at once.
                export APARTE_CLAUDE_DIR="''${APARTE_CLAUDE_DIR:-$APARTE_SRC/.claude-auth}"
                mkdir -p "$APARTE_CLAUDE_DIR"
                gitconfig="''${XDG_CONFIG_HOME:-$HOME/.config}/git/config"
                [[ -f $gitconfig ]] || gitconfig="$HOME/.gitconfig"
                export APARTE_GITCONFIG="''${APARTE_GITCONFIG:-$gitconfig}"
                export APARTE_HM_DIR="''${APARTE_HM_DIR:-''${XDG_CONFIG_HOME:-$HOME/.config}/home-manager}"
                echo "booting the aparte dev microvm (sharing $APARTE_SRC at /home/dev/aparte, $APARTE_CLAUDE_DIR at /home/dev/.claude)"
                exec nix run --impure "$APARTE_SRC#dev-vm"
              fi
            '';
          };
        };
      }
    );
}
