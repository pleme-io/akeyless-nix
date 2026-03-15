# NixOS module for akeyless-nix
#
# Fetches secrets from Akeyless at activation time and writes them to /run/secrets/.
# Supports ramfs mounting, owner/group, and neededForUsers (pre-sysusers secrets).
{ config, lib, pkgs, ... }:
let
  cfg = config.akeyless;

  manifestFile = pkgs.writeText "akeyless-manifest.json" (builtins.toJSON {
    secrets = lib.mapAttrsToList (name: secret: {
      akeyless_path = name;
      file_path = secret.path;
      mode = secret.mode;
      owner = secret.owner;
      group = secret.group;
    }) cfg.secrets;

    templates = lib.mapAttrsToList (name: tmpl: {
      inherit name;
      content = tmpl.content;
      file_path = tmpl.path;
      mode = tmpl.mode;
      owner = tmpl.owner;
      group = tmpl.group;
    }) cfg.templates;

    generations_dir = "/run/akeyless-nix.d";
    symlink_path = "/run/akeyless-nix";
    keep_generations = cfg.keepGenerations;
  });

in {
  options.akeyless = {
    enable = lib.mkEnableOption "Akeyless secret management for NixOS";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.akeyless-install-secrets;
      description = "The akeyless-install-secrets package";
    };

    secrets = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          path = lib.mkOption {
            type = lib.types.str;
            description = "Target file path (default: /run/akeyless-nix/{name})";
            default = "";
          };
          mode = lib.mkOption {
            type = lib.types.str;
            default = "0400";
          };
          owner = lib.mkOption {
            type = lib.types.str;
            default = "root";
          };
          group = lib.mkOption {
            type = lib.types.str;
            default = "root";
          };
          neededForUsers = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "If true, decrypt before user creation (for password hashes)";
          };
          restartUnits = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
            description = "Systemd units to restart when this secret changes";
          };
        };
      });
      default = {};
    };

    templates = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          path = lib.mkOption { type = lib.types.str; };
          content = lib.mkOption { type = lib.types.str; };
          mode = lib.mkOption { type = lib.types.str; default = "0400"; };
          owner = lib.mkOption { type = lib.types.str; default = "root"; };
          group = lib.mkOption { type = lib.types.str; default = "root"; };
        };
      });
      default = {};
    };

    placeholder = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      readOnly = true;
      default = {};
    };

    keepGenerations = lib.mkOption {
      type = lib.types.int;
      default = 2;
    };
  };

  config = lib.mkIf cfg.enable {
    akeyless.placeholder = lib.mapAttrs (name: _:
      "<AKEYLESS:${builtins.hashString "sha256" name}:PLACEHOLDER>"
    ) cfg.secrets;

    # Default paths: /run/akeyless-nix/{sanitized-name}
    akeyless.secrets = lib.mapAttrs (name: secret:
      secret // lib.optionalAttrs (secret.path == "") {
        path = "/run/akeyless-nix/${lib.replaceStrings ["/"] ["-"] (lib.removePrefix "/" name)}";
      }
    ) cfg.secrets;

    # Activation script — runs after users/groups are created
    system.activationScripts.akeylessSecrets = lib.stringAfter [ "specialfs" "users" "groups" ] ''
      echo "akeyless-nix: installing secrets..."
      ${cfg.package}/bin/akeyless-install-secrets install ${manifestFile} --ignore-passwd || \
        echo "akeyless-nix: WARNING — secret installation failed"
    '';
  };
}
