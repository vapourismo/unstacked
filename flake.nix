{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};

      frameworks = pkgs.darwin.apple_sdk.frameworks;

      cargoManifest = builtins.fromTOML (builtins.readFile (self + /Cargo.toml));
    in {
      packages.default = pkgs.rustPlatform.buildRustPackage {
        pname = "unstacked";
        version = cargoManifest.package.version;

        src = pkgs.nix-gitignore.gitignoreSource [] self;

        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = [
          pkgs.pkg-config
        ];

        buildInputs = with pkgs; [
          openssl.dev
          libgpg-error
          gpgme.dev
          libgit2
        ];
      };

      devShell = pkgs.mkShell {
        name = "dev";

        inputsFrom = [
          self.packages.${system}.default
        ];

        packages = with pkgs;
          [
            nil
            alejandra
            clippy
            rust-analyzer
            rustfmt
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            frameworks.SystemConfiguration
          ];
      };
    });
}
