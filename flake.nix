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
          cargoExtraArgs = "--locked --workspace";
          strictDeps = true;
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        commonArgsAndCargoArtifacts = commonArgs // {
          inherit cargoArtifacts;
        };

        qemuPluginReadySrcDir = ./qemu-plugin-ready;

        qemu-plugin-ready = craneLib.buildPackage (
          commonArgsAndCargoArtifacts
          // {
            inherit (craneLib.crateNameFromCargoToml { src = qemuPluginReadySrcDir; }) pname version;

            cargoExtraArgs = "--locked -p qemu-plugin-ready";

            patches = [
              (pkgs.writeText "empty-src-main-rs.patch" ''
                diff --git a/src/main.rs b/src/main.rs
                new file mode 100644
              '')
            ];

            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./.cargo/config.toml
                ./Cargo.toml
                ./Cargo.lock
                (craneLib.fileset.commonCargoSources qemuPluginReadySrcDir)
              ];
            };
          }
        );

        libraryPathEnvVar =
          if pkgs.stdenv.hostPlatform.isDarwin then "DYLD_FALLBACK_LIBRARY_PATH" else "LD_LIBRARY_PATH";

        qemu-wrapper = craneLib.buildPackage (
          commonArgsAndCargoArtifacts
          // {
            cargoExtraArgs = "--locked";
            nativeBuildInputs = [ pkgs.makeBinaryWrapper ];

            postInstall = ''
              wrapProgram $out/bin/qemu-wrapper \
                --prefix PATH : ${lib.makeBinPath [ pkgs.qemu ]} \
                --prefix ${libraryPathEnvVar} : ${lib.makeLibraryPath [ qemu-plugin-ready ]}
            '';

            src = lib.cleanSourceWith {
              filter =
                name: type:
                (name != qemuPluginReadySrcDir)
                || !(lib.assertMsg (
                  type == "directory"
                ) "qemuPluginReadySrcDir: '${qemuPluginReadySrcDir}' has non-directory type: '${type}'");
              src = craneLib.cleanCargoSource commonArgs.src;
            };
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
          inherit qemu-plugin-ready qemu-wrapper; # check build

          formatting = treefmt.check self;
          qemu-wrapper-clippy = craneLib.cargoClippy commonArgsAndCargoArtifacts;
          qemu-wrapper-doc = craneLib.cargoDoc commonArgsAndCargoArtifacts;
        };

        packages = {
          inherit qemu-plugin-ready qemu-wrapper;
          default = qemu-wrapper;
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
