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

        qemu-wrapper = craneLib.buildPackage (
          commonArgsAndCargoArtifacts
          // {
            postInstall = ''
              wrapProgram $out/bin/qemu-wrapper --prefix PATH : ${lib.makeBinPath [ pkgs.qemu ]}
            '';
            nativeBuildInputs = [ pkgs.makeBinaryWrapper ];
          }
        );

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
          inherit qemu-wrapper; # check build

          formatting = treefmt.check self;
          qemu-wrapper-clippy = craneLib.cargoClippy commonArgsAndCargoArtifacts;
          qemu-wrapper-doc = craneLib.cargoDoc commonArgsAndCargoArtifacts;
        };

        packages = {
          inherit qemu-wrapper;
          default = qemu-wrapper;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks."${system}"; # inherit inputs
        };

        formatter = treefmt.wrapper;
      }
    );
}
