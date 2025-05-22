{
  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    nixpkgs-mozilla = {
      url = "github:mozilla/nixpkgs-mozilla";
      flake = false;
    };
  };

  outputs = { self, flake-utils, naersk, nixpkgs, nixpkgs-mozilla }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = (import nixpkgs) {
          inherit system;
          overlays = [
            (import nixpkgs-mozilla)
          ];
        };

        # toolchain = (pkgs.rustChannelOf {
        #   rustToolchain = ./rust-toolchain.toml;
        #   sha256 = "sha256-Xb/lE3DAZPNhrxCqtWiCfKBTHuWl0e0c7ZYbqrzjFeI=";
        # }).rust;

        naersk' = pkgs.callPackage naersk {
          # cargo = toolchain;
          # rustc = toolchain;
        };

      in {
        # For `nix build` & `nix run`:
        defaultPackage = naersk'.buildPackage {
          src = ./.;
        };

        # For `nix develop`:
        devShell = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ rustc cargo clippy rustfmt ];
        };
      }
    );
}
