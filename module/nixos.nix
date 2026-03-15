# NixOS module for akeyless-nix
#
# Fetches secrets from Akeyless at activation time and writes them to /run/secrets/.
# Supports ramfs mounting, owner/group, uid/gid, neededForUsers (pre-sysusers secrets),
# and restartUnits/reloadUnits for systemd service integration.
{ config, lib, pkgs, ... }:
let
  cfg = config.akeyless;
  alib = import ./lib.nix { inherit lib; };

  manifestFile = alib.buildManifest {
    inherit pkgs cfg;
    generationsDir = "/run/akeyless-nix.d";
    symlinkPath = "/run/akeyless-nix";
  };

  allRestartUnits = alib.collectRestartUnits cfg.secrets;
  allReloadUnits = alib.collectReloadUnits cfg.secrets;

in {
  options.akeyless = {
    enable = lib.mkEnableOption "Akeyless secret management for NixOS";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.akeyless-install-secrets;
      description = "The akeyless-install-secrets package.";
    };

    secrets = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          path = lib.mkOption {
            type = lib.types.str;
            description = "Target file path (default: /run/akeyless-nix/{name}).";
            default = "";
          };
          mode = lib.mkOption {
            type = lib.types.str;
            default = "0400";
            description = "File permission mode (octal string, e.g. \"0400\").";
          };
          owner = lib.mkOption {
            type = lib.types.str;
            default = "root";
            description = "File owner name.";
          };
          group = lib.mkOption {
            type = lib.types.str;
            default = "root";
            description = "File group name.";
          };
          uid = lib.mkOption {
            type = lib.types.nullOr lib.types.int;
            default = null;
            description = "File owner UID (alternative to owner name). Takes precedence over owner.";
          };
          gid = lib.mkOption {
            type = lib.types.nullOr lib.types.int;
            default = null;
            description = "File group GID (alternative to group name). Takes precedence over group.";
          };
          neededForUsers = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = "If true, decrypt before user creation (for password hashes).";
          };
          restartUnits = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
            description = "Systemd units to restart when this secret changes.";
          };
          reloadUnits = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [];
            description = "Systemd units to reload when this secret changes.";
          };
        };
      });
      default = {};
    };

    templates = lib.mkOption {
      type = lib.types.attrsOf alib.templateSubmodule;
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

    ignorePasswd = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Pass --ignore-passwd to skip owner/group lookups (useful in CI/dry-run).";
    };
  };

  config = lib.mkIf cfg.enable {
    akeyless.placeholder = alib.mkPlaceholders cfg.secrets;

    # Activation script -- runs after users/groups are created
    system.activationScripts.akeylessSecrets = lib.stringAfter [ "specialfs" "users" "groups" ] ''
      echo "akeyless-nix: installing secrets..."
      ${cfg.package}/bin/akeyless-install-secrets install \
        ${lib.optionalString cfg.ignorePasswd "--ignore-passwd "}\
        ${manifestFile} || \
        echo "akeyless-nix: WARNING -- secret installation failed"

      # Restart units that depend on changed secrets.
      # The binary writes secrets atomically; we trigger restarts unconditionally
      # on activation since we cannot yet diff generations from the activation script.
      ${lib.concatMapStringsSep "\n" (unit: ''
        echo "akeyless-nix: restarting ${unit}"
        systemctl restart ${lib.escapeShellArg unit} 2>/dev/null || true
      '') allRestartUnits}

      ${lib.concatMapStringsSep "\n" (unit: ''
        echo "akeyless-nix: reloading ${unit}"
        systemctl reload ${lib.escapeShellArg unit} 2>/dev/null || true
      '') allReloadUnits}
    '';
  };
}
