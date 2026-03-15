{
  description = "akeyless-nix — Akeyless secret management for Nix (drop-in sops-nix replacement)";

  nixConfig = {
    allow-import-from-derivation = true;
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    crate2nix.url = "github:nix-community/crate2nix";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    substrate = {
      url = "github:pleme-io/substrate";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.fenix.follows = "fenix";
    };
    devenv = {
      url = "github:cachix/devenv";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    crate2nix,
    flake-utils,
    fenix,
    substrate,
    devenv,
    ...
  }:
    (import "${substrate}/lib/rust-tool-release-flake.nix" {
      inherit nixpkgs crate2nix flake-utils fenix devenv;
    }) {
      toolName = "akeyless-install-secrets";
      src = self;
      repo = "pleme-io/akeyless-nix";
    }
    // {
      homeManagerModules.default = import ./module;
      darwinModules.default = import ./module/darwin.nix;
    };
}
