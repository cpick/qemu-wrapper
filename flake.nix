{
  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
  };

  outputs = { self, flake-utils, naersk, nixpkgs }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages."${system}";
      in {
        packages.default = naersk.lib."${system}".buildPackage {
          src = self;
        };

        devShells.default = pkgs.mkShell {
          nativeBuildInputs = [
            pkgs.bacon
            pkgs.cargo
            pkgs.cargo-watch
            pkgs.clippy
            pkgs.lldb_18
            pkgs.nil
            pkgs.rust-analyzer
            pkgs.qemu
            pkgs.rustc
            pkgs.rustfmt
            pkgs.taplo
          ];
          RUST_SRC_PATH = pkgs.rust.packages.stable.rustPlatform.rustLibSrc;
        };
      }
    );
}
