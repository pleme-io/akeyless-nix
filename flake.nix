{
  description = "akeyless-nix — Akeyless secret management for Nix (drop-in sops-nix replacement)";

  inputs.substrate.url = "github:pleme-io/substrate";

  # The substrate-emitted module trio (programs.akeyless-install-secrets…)
  # is the WRONG shape for this flake. akeyless-nix ships a hand-authored
  # HM/NixOS/Darwin module trio under `./module` that declares
  # `options.akeyless` — the namespace blackmatter-secrets's akeyless
  # backend setter writes to. The override below replaces substrate's
  # auto-generated trio with the hand-authored one. Cannot collapse to
  # the 3-line `substrate.rust.tool { src = ./.; }` form because the
  # module identity is fundamentally non-substrate-shaped — this is the
  # documented exception in the central-control-plane pattern.
  outputs = { substrate, ... }:
    substrate.rust.tool { src = ./.; }
    // {
      homeManagerModules.default = import ./module;
      darwinModules.default = import ./module/darwin.nix;
      nixosModules.default = import ./module/nixos.nix;
    };
}
