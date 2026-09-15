# The dev environment as a NixOS guest, run through microvm.nix's declaredRunner.
#
# `srcDir` is the host path shared in at /home/dev/aparte. `claudeDir` is the project's own
# .claude-auth, shared in at /home/dev/.claude so sessions, skills, and memory survive a VM
# restart instead of resetting every boot. Under a pure evaluation they are null and the
# guest carries no share at all, which is why flake.nix sets APARTE_SRC / APARTE_CLAUDE_DIR
# and re-enters with --impure: a 9p share has to name a host path at evaluation time, and
# only the caller knows where it is. `gitConfig` is the host's global git config, carried in
# by value for the same reason, and `hmDir` the host's home-manager config, imported.
{ rust-overlay, srcDir, claudeDir, gitConfig, hmDir }:

{ pkgs, lib, ... }:

let
  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;
  shareMountPoints =
    lib.optional (srcDir != null) "/home/dev/aparte"
    ++ lib.optional (claudeDir != null) "/home/dev/.claude";
in
{
  nixpkgs.overlays = [ (import rust-overlay) ];
  # claude-code is unfree. The devShells get this from their own `import nixpkgs`; a
  # nixosSystem builds its own pkgs and inherits none of it.
  nixpkgs.config.allowUnfree = true;

  microvm = {
    hypervisor = "qemu";
    vcpu = 4;
    mem = 4096;

    # user networking needs nothing of the host -- no tap device, no bridge, no root. Aparte
    # only ever dials out to XMPP servers; nothing in the guest listens for inbound
    # connections, so there is no forwardPorts entry to add.
    interfaces = [{
      type = "user";
      id = "vm-aparte";
      mac = "02:00:00:ca:fe:02";
    }];

    # The store travels as a disk image rather than a share. Sharing /nix/store would be
    # virtiofs, and virtiofs needs a virtiofsd daemon started alongside qemu -- the runner
    # ships `virtiofsd-run` as a *separate* supervisord process and `microvm-run` mentions
    # it nowhere, so a bare `nix run` dies on a missing socket. That orchestration belongs
    # to a NixOS host module, not to a dev shell.
    storeOnDisk = true;

    # storeOnDisk alone leaves /nix/store read-only (erofs), which breaks anything that
    # calls `nix-store --realise` at runtime -- home-manager's generation switch does,
    # unconditionally, even to re-realise a path already valid in the store, and that call
    # needs to create /nix/store/.links. Without this overlay, home-manager-dev.service
    # fails on every boot with "creating directory \"/nix/store/.links\": Read-only file
    # system", and the git identity imported via hmDir below never actually lands in the
    # guest. tmpfs is enough for the writable upper layer: it only holds the small writes
    # realise/nix-env make, not a copy of the store, and losing it on reboot is fine since
    # activation reruns from the same storeDisk regardless.
    writableStoreOverlay = "/nix/.rw-store";

    # 9p rather than virtiofs for the same reason: qemu serves it in-process, so nothing
    # has to be started first. It is the slower of the two, which is why CARGO_TARGET_DIR
    # below points off it -- what crosses 9p is source reads, not the build tree.
    #
    # securityModel defaults to "none", documented as passing the real host uid/gid through.
    # Measured against qemu 11.1 under the `-sandbox on` microvm adds whenever the qemu
    # build and the interfaces allow it, both shares instead arrive entirely as uid 0,
    # unwritable, across access=any/client and 9p2000.L/.u alike. "mapped" keeps
    # guest-visible ownership in a host-side xattr, which is what lets the chown in
    # `fix-share-ownership` below stick. Its price is that files created *in the guest* land
    # on the host at 0600, owned by whoever ran qemu, with the guest's mode in the xattr.
    shares =
      lib.optional (srcDir != null) {
        source = srcDir;
        mountPoint = "/home/dev/aparte";
        tag = "src";
        proto = "9p";
        securityModel = "mapped";
      }
      ++ lib.optional (claudeDir != null) {
        source = claudeDir;
        mountPoint = "/home/dev/.claude";
        tag = "claude";
        proto = "9p";
        securityModel = "mapped";
      };

    # Aparte's own state (storage.sqlite, config.toml, logs) is small and lives under the
    # guest's $HOME, so it needs no volume of its own. The one thing worth persisting across
    # boots is the build tree: without a volume, CARGO_TARGET_DIR would sit on the writable
    # store overlay's tmpfs and rebuild from scratch every boot.
    volumes = [{
      image = "aparte-dev-var.img";
      mountPoint = "/var";
      size = 32768;
    }];
  };

  # cargo needs a real CA bundle to fetch crates over HTTPS; the guest's own nixpkgs
  # provides it the same way the host devShell's sslHook does. protobuf/PROTOC is here for
  # the OMEMO stack's prost-generated code.
  environment.variables = {
    RUST_LOG = "info";
    CARGO_TARGET_DIR = "/var/cache/cargo/target";
    # claude-code resolves .claude.json against CLAUDE_CONFIG_DIR rather than $HOME, so
    # aiming it at the share is what carries onboarding, project trust and MCP registrations
    # across a reboot. A symlink from $HOME would not: that file is replaced by rename.
    CLAUDE_CONFIG_DIR = "/home/dev/.claude";
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    NIX_SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    CARGO_HTTP_CAINFO = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    OPENSSL_NO_VENDOR = "1";
    PROTOC = "${pkgs.protobuf}/bin/protoc";
  };

  environment.systemPackages = [ rustToolchain ] ++ (with pkgs; [
    rust-analyzer
    gcc
    pkg-config
    openssl
    openssl.dev
    sqlite
    cacert
    protobuf
    claude-code
    neovim
    git
    pre-commit
    asciinema
  ]);

  users.users.dev = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    password = "";
  };
  security.sudo.wheelNeedsPassword = false;
  services.getty.autologinUser = "dev";

  systemd.tmpfiles.rules = [
    "d /var/cache/cargo 0755 dev users -"
  ];

  # A file whose xattr has never been written arrives as uid 0, so each share is claimed for
  # `dev` once per boot. Only entries able to carry the xattr are visited: the kernel refuses
  # user.* on a symlink and on anything the writer holds no write bit for, and skipping those
  # costs nothing -- a 0444 git object stays world-readable, a symlink resolves through its
  # target. What remains unclaimed is reported and tolerated, because a share that cannot be
  # claimed in full must not stop the next share from being claimed at all.
  systemd.services.fix-share-ownership = lib.mkIf (shareMountPoints != [ ]) {
    description = "chown mapped 9p shares to dev";
    wantedBy = [ "multi-user.target" ];
    # Both getty flavours are ordered After systemd-user-sessions.service, which is the only
    # hook that holds on either arch: the console unit is serial-getty@ttyS0 on x86_64 and
    # @ttyAMA0 on aarch64. getty.target alone would leave them unordered siblings, and the
    # autologin shell would race a share it may still see as root's.
    before = [ "systemd-user-sessions.service" ];
    unitConfig.RequiresMountsFor = shareMountPoints;
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    script = lib.concatMapStringsSep "\n" (p: ''
      find ${lib.escapeShellArg p} \( -type d -o -type f \) -perm -u+w -print0 |
        xargs -0r chown dev:users ||
        echo "entries under ${p} left unclaimed for dev"
    '') shareMountPoints;
  };

  # git in the guest is a different user on a different machine, but the same person: the
  # host's identity and aliases come along so commits made here are not attributed to nobody.
  # Only when home-manager is not already carrying it: the host's config is itself
  # home-manager output, and git accumulates multi-valued keys across scopes rather than
  # overriding them, so importing both would run a credential helper twice.
  environment.etc."gitconfig" = lib.mkIf (gitConfig != null && hmDir == null) { text = gitConfig; };

  # useGlobalPkgs takes the guest's own pkgs rather than standing up a second nixpkgs, at
  # the price of the host config's nixpkgs.config and overlays being ignored -- a home.nix
  # that pulls a package from its own overlay fails here with a missing attribute.
  # useUserPackages routes home.packages through users.users.dev, so activation never writes
  # a nix profile against the read-only store. The host config names its own user and home,
  # which `dev` is not.
  home-manager = lib.mkIf (hmDir != null) {
    useGlobalPkgs = true;
    useUserPackages = true;
    users.dev = { lib, ... }: {
      imports = [ (/. + (hmDir + "/home.nix")) ];
      home.username = lib.mkForce "dev";
      home.homeDirectory = lib.mkForce "/home/dev";
    };
  };

  networking.hostName = "aparte-dev";
  system.stateVersion = "25.05";
}
