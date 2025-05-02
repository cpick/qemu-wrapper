{
  inputs = {
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";

    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      crane,
      flake-utils,
      nixpkgs,
      treefmt-nix,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages."${system}";
        inherit (pkgs) lib;

        craneLib = crane.mkLib pkgs;
        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        commonArgsAndCargoArtifacts = commonArgs // {
          inherit cargoArtifacts;
        };

        qemu-plugin-ready = craneLib.buildPackage (commonArgsAndCargoArtifacts);

        treefmt =
          (treefmt-nix.lib.evalModule pkgs {
            projectRootFile = "flake.nix";

            programs = {
              nixfmt.enable = true;
              taplo.enable = true;
              rustfmt = {
                edition = (lib.importTOML ./Cargo.toml).package.edition;
                enable = true;
              };
            };
          }).config.build;
      in
      {
        checks = {
          inherit qemu-plugin-ready; # check build

          formatting = treefmt.check self;
          qemu-plugin-ready-clippy = craneLib.cargoClippy commonArgsAndCargoArtifacts;
          qemu-plugin-ready-doc = craneLib.cargoDoc commonArgsAndCargoArtifacts;
        };

        packages = {
          inherit qemu-plugin-ready;
          default = qemu-plugin-ready;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks."${system}"; # inherit inputs

          packages = [
            # manual tools/debugging
            pkgs.qemu
          ];
        };

        formatter = treefmt.wrapper;
      }
    );
}
