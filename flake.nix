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
        commonArgsAndCargoArtifacts = commonArgs // {
          inherit cargoArtifacts;
        };

        qemu-wrapper = craneLib.buildPackage (
          commonArgsAndCargoArtifacts
          // {
            postInstall = ''
              wrapProgram $out/bin/qemu-wrapper --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.qemu ]}
            '';
            nativeBuildInputs = [ pkgs.makeBinaryWrapper ];
          }
        );
      in
      {
        checks = {
          inherit qemu-wrapper; # check build
          qemu-wrapper-clippy = craneLib.cargoClippy commonArgsAndCargoArtifacts;
          qemu-wrapper-doc = craneLib.cargoDoc commonArgsAndCargoArtifacts;

          qemu-wrapper-fmt = craneLib.cargoFmt {
            inherit src;
          };

          qemu-wrapper-toml-fmt = craneLib.taploFmt {
            src = pkgs.lib.sources.sourceFilesBySuffices src [ ".toml" ];
          };
        };

        packages = {
          inherit qemu-wrapper;
          default = qemu-wrapper;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks."${system}"; # inherit inputs
        };

        formatter = pkgs.nixfmt-rfc-style;
      }
    );
}
