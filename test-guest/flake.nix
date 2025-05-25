{
  inputs = {
    crane.url = "github:ipetkov/crane";

    fenix = {
      inputs.nixpkgs.follows = "nixpkgs";
      url = "github:nix-community/fenix";
    };

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
      fenix,
      flake-utils,
      nixpkgs,
      treefmt-nix,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages."${system}";
        inherit (pkgs) lib;

        cargoConfig = (lib.importTOML ./.cargo/config.toml);

        toolchain =
          let
            fenixPkgs = fenix.packages."${system}";
          in
          fenixPkgs.combine [
            fenixPkgs.stable.cargo
            fenixPkgs.stable.rustc
            fenixPkgs.targets."${cargoConfig.build.target}".stable.rust-std
          ];

        craneLib = ((crane.mkLib pkgs).overrideToolchain toolchain);
        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./config
            (craneLib.fileset.commonCargoSources ./.)
          ];
        };

        commonArgs = {
          inherit src;
          doCheck = false;
          strictDeps = true;
          CARGO_BUILD_TARGET = cargoConfig.build.target;
        };

        cargoArtifacts = craneLib.buildDepsOnly (
          commonArgs
          // {
            dummyrs = pkgs.writeText "dummy.rs" ''
              #![cfg_attr(target_os = "uefi", no_std)]
              #![cfg_attr(target_os = "uefi", no_main)]

              #[cfg(target_os = "uefi")]
              #[uefi::entry]
              fn main() -> uefi::Status {
                  uefi::Status::SUCCESS
              }

              #[cfg(not(target_os = "uefi"))]
              fn main() {}
            '';
          }
        );
        commonArgsAndCargoArtifacts = commonArgs // {
          inherit cargoArtifacts;
        };

        test-guest = craneLib.buildPackage commonArgsAndCargoArtifacts;

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
          inherit test-guest; # check build

          formatting = treefmt.check self;
        };

        packages = {
          inherit test-guest;
          default = test-guest;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks."${system}"; # inherit inputs

          packages = [
            # runtime dependencies
            pkgs.qemu
          ];
        };

        formatter = treefmt.wrapper;
      }
    );
}
