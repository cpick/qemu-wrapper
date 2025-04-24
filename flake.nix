{
  inputs = {
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    {
      self,
      crane,
      flake-utils,
      nixpkgs,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages."${system}";

        craneLib = crane.mkLib pkgs;
        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        qemu-wrapper = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            postInstall = ''
              wrapProgram $out/bin/qemu-wrapper --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.qemu ]}
            '';
            nativeBuildInputs = [ pkgs.makeBinaryWrapper ];
          }
        );
      in
      {
        checks = {
          inherit qemu-wrapper; # build as part of `nix flake check` for convenience

          qemu-wrapper-clippy = craneLib.cargoClippy (
            commonArgs
            // {
              inherit cargoArtifacts;
            }
          );

          qemu-wrapper-doc = craneLib.cargoDoc (
            commonArgs
            // {
              inherit cargoArtifacts;
            }
          );

          qemu-wrapper-fmt = craneLib.cargoFmt {
            inherit src;
          };

          qemu-wrapper-toml-fmt = craneLib.taploFmt {
            src = pkgs.lib.sources.sourceFilesBySuffices src [ ".toml" ];
            # taploExtraArgs = "--config ./taplo.toml";
          };
        };

        packages = {
          default = qemu-wrapper;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = qemu-wrapper;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks."${system}"; # inherit inputs
        };

        formatter = pkgs.nixfmt-rfc-style;
      }
    );
}
