{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        notecli = pkgs.rustPlatform.buildRustPackage {
          pname = "notecli";
          version = "0.1.0";
          src = ./.;
          cargoHash = "sha256-8nNtS9QVLxYPWx75oSvvy+7VYYq8e+B/abIbB5qF7Gc=";
          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs =
            with pkgs;
            [ openssl ]
            ++ lib.optionals stdenv.hostPlatform.isDarwin [
              darwin.apple_sdk.frameworks.Security
              darwin.apple_sdk.frameworks.SystemConfiguration
            ];
          buildNoDefaultFeatures = true;
        };
      in
      {
        packages.default = notecli;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ notecli ];
          packages = with pkgs; [
            rust-analyzer
            clippy
          ];
        };
      }
    );
}
