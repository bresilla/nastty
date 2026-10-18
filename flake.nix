{
  description = "nastty: a single-binary NAS server and terminal UI for bcachefs";

  nixConfig = {
    extra-substituters = [ "https://bresilla.cachix.org" ];
    extra-trusted-public-keys = [
      "bresilla.cachix.org-1:jpce/F2++w5NrhkQ19Lptd21Ck7mXT+FWC1ramXLipA="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?rev=4c1018dae018162ec878d42fec712642d214fdfa";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { nixpkgs, flake-utils, ... }:
    let
      manifest = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package;
    in
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;
        # The revision Cargo.toml pins the nasty crates to.
        nastyUpstream = pkgs.fetchFromGitHub {
          owner = "nasty-project";
          repo = "nasty";
          rev = "4ad3fa304b57b8ce82774a36693aa87769d1478b";
          hash = "sha256-EtpXJNzNQ5m8MIN4J7UzR/c3kw8bN5h9ny2LuMgoicc=";
        };
        nastty = pkgs.rustPlatform.buildRustPackage {
          pname = manifest.name;
          inherit (manifest) version;
          src = lib.cleanSource ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            # The upstream nasty crates are git dependencies, pinned by revision in Cargo.lock.
            allowBuiltinFetchGit = true;
          };
          cargoBuildFlags = [ "--bin" manifest.name ];
          # nasty-system embeds two files from its repository root, which vendoring drops;
          # they go where its include_str! paths resolve, beside cargo-vendor-dir.
          preBuild = ''
            mkdir -p "$NIX_BUILD_TOP/nixos/system-flake"
            cp ${nastyUpstream}/flake.nix "$NIX_BUILD_TOP/flake.nix"
            cp ${nastyUpstream}/nixos/system-flake/flake.nix.template "$NIX_BUILD_TOP/nixos/system-flake/"
          '';
          # The suite runs in CI; the package proves only that the binary starts.
          doCheck = false;
          doInstallCheck = true;
          installCheckPhase = ''
            runHook preInstallCheck
            "$out/bin/${manifest.name}" --version
            "$out/bin/${manifest.name}" serve --help > /dev/null
            "$out/bin/${manifest.name}" tui --help > /dev/null
            runHook postInstallCheck
          '';
          meta = {
            inherit (manifest) description;
            homepage = "https://github.com/bresilla/nastty";
            license = lib.licenses.mit;
            mainProgram = manifest.name;
            platforms = lib.platforms.linux;
          };
        };
      in
      {
        packages = {
          inherit nastty;
          default = nastty;
        };

        apps.default = {
          type = "app";
          program = lib.getExe nastty;
          meta.description = "Run nastty";
        };

        checks = { inherit nastty; };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rustc
            cargo
            rustfmt
            clippy
            rust-analyzer
            git-cliff
            pkg-config
            # The NAS tools nastty shells out to; the NFS and SMB daemons and the
            # bcachefs kernel module come from the host.
            bcachefs-tools
            nfs-utils
            samba
            smartmontools
          ];
        };
      }
    );
}
