{
  inputs = {
    crane.url = "github:ipetkov/crane";

    fenix = {
      inputs.nixpkgs.follows = "nixpkgs";
      url = "github:nix-community/fenix";
    };

    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";

    test-guest = {
      url = "path:test-guest";
      inputs = {
        crane.follows = "crane";
        fenix.follows = "fenix";
        flake-utils.follows = "flake-utils";
        nixpkgs.follows = "nixpkgs";
        treefmt-nix.follows = "treefmt-nix";
      };
    };

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
      test-guest,
      treefmt-nix,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages."${system}";
        inherit (pkgs) lib;

        toolchain =
          let
            fenixPkgs = fenix.packages."${system}";
          in
          fenixPkgs.combine [
            fenixPkgs.stable.cargo
            fenixPkgs.stable.clippy
            fenixPkgs.stable.rustc
            fenixPkgs.targets.x86_64-unknown-uefi.stable.rust-std
          ];

        craneLib = ((crane.mkLib pkgs).overrideToolchain toolchain);
        src = craneLib.cleanCargoSource ./.;
        rootCargoSources = [
          ./.cargo/config.toml
          ./Cargo.lock
          ./Cargo.toml
        ];

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

            cargoExtraArgs = "--locked --package qemu-plugin-ready";

            patches = [
              (pkgs.writeText "empty-src-main-rs.patch" ''
                diff --git a/src/main.rs b/src/main.rs
                new file mode 100644
              '')
            ];

            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions (
                rootCargoSources
                ++ [
                  (craneLib.fileset.commonCargoSources qemuPluginReadySrcDir)
                ]
              );
            };
          }
        );

        libraryPathEnvVar =
          if pkgs.stdenv.hostPlatform.isDarwin then "DYLD_FALLBACK_LIBRARY_PATH" else "LD_LIBRARY_PATH";

        qemu-wrapper = craneLib.buildPackage (
          commonArgsAndCargoArtifacts
          // {
            cargoExtraArgs = "--locked --package qemu-wrapper";
            nativeBuildInputs = [
              pkgs.makeBinaryWrapper
              pkgs.qemu
            ];

            postInstall = ''
              wrapProgram $out/bin/qemu-wrapper \
                --prefix PATH : ${lib.makeBinPath [ pkgs.qemu ]} \
                --prefix ${libraryPathEnvVar} : ${lib.makeLibraryPath [ qemu-plugin-ready ]}
            '';

            patches = [
              (pkgs.writeText "empty-plugin-lib-rs.patch" ''
                diff --git a/qemu-plugin-ready/src/lib.rs b/qemu-plugin-ready/src/lib.rs
                new file mode 100644
              '')
            ];

            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions (
                rootCargoSources
                ++ [
                  ./test-guest/config
                  (craneLib.fileset.cargoTomlAndLock qemuPluginReadySrcDir)
                  (craneLib.fileset.commonCargoSources ./src)
                  (craneLib.fileset.commonCargoSources ./tests)
                ]
              );
            };

            QEMU_PLUGIN_PATH = lib.makeLibraryPath [ qemu-plugin-ready ];
            TEST_GUEST_PATH = lib.makeBinPath [ test-guest.packages."${system}".test-guest ];
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

          qemu-wrapper-clippy = craneLib.cargoClippy (
            commonArgsAndCargoArtifacts
            // {
              src = lib.fileset.toSource {
                root = ./.;
                fileset = lib.fileset.unions (
                  rootCargoSources
                  ++ [
                    ./test-guest/config
                    (craneLib.fileset.commonCargoSources qemuPluginReadySrcDir)
                    (craneLib.fileset.commonCargoSources ./src)
                    (craneLib.fileset.commonCargoSources ./tests)
                  ]
                );
              };
            }
          );

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
