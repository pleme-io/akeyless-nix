# Home-manager module for akeyless-nix secret management.
#
# Drop-in replacement for sops-nix: declares secrets and templates,
# generates a JSON manifest at eval time, and runs akeyless-install-secrets
# at activation time to fetch and write secret files.
{ config, lib, pkgs, ... }:
let
  cfg = config.akeyless;
  alib = import ./lib.nix { inherit lib; };

  manifestFile = alib.buildManifest {
    inherit pkgs cfg;
    generationsDir = cfg.defaultSecretsMountPoint;
    symlinkPath = cfg.defaultSymlinkPath;
  };
in {
  options.akeyless = {
    enable = lib.mkEnableOption "Akeyless secret management (replaces sops-nix)";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.akeyless-install-secrets;
      description = "The akeyless-install-secrets package.";
    };

    defaultSecretsMountPoint = lib.mkOption {
      type = lib.types.str;
      default = "${config.xdg.dataHome}/akeyless-nix/generations";
      description = "Directory for secret generations.";
    };

    defaultSymlinkPath = lib.mkOption {
      type = lib.types.str;
      default = "${config.xdg.dataHome}/akeyless-nix/secrets";
      description = "Symlink path for current generation + auto-generated secret paths.";
    };

    secrets = lib.mkOption {
      type = lib.types.attrsOf alib.secretSubmodule;
      default = {};
      description = "Secrets to fetch from Akeyless. Keys are Akeyless paths.";
    };

    templates = lib.mkOption {
      type = lib.types.attrsOf alib.templateSubmodule;
      default = {};
      description = "Templates with secret placeholder substitution.";
    };

    placeholder = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      readOnly = true;
      default = {};
      description = "Auto-generated placeholders for templates.";
    };

    keepGenerations = lib.mkOption {
      type = lib.types.int;
      default = 2;
      description = "Number of secret generations to keep.";
    };

    ignorePasswd = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Skip owner/group lookups (for CI/dry-run).";
    };

    templateEngine = lib.mkOption {
      type = lib.types.enum [ "placeholder" "igata" ];
      default = "placeholder";
      description = "Template engine for rendering (placeholder = legacy hash-based, igata = MiniJinja).";
    };
  };

  config = lib.mkIf cfg.enable {
    akeyless.placeholder = alib.mkPlaceholders cfg.secrets;
    xdg.enable = true;

    home.activation.akeylessSecrets = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
      ${cfg.package}/bin/akeyless-install-secrets install \
        ${lib.optionalString cfg.ignorePasswd "--ignore-passwd "}\
        --template-engine ${cfg.templateEngine} \
        ${manifestFile} \
        || echo "akeyless-nix: WARNING — secret installation failed (offline?)"
    '';
  };
}
