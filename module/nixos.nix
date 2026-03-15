# NixOS module for akeyless-nix
#
# Fetches secrets from Akeyless at activation time and writes them to /run/secrets/.
# Supports ramfs mounting, owner/group, uid/gid, neededForUsers (pre-sysusers secrets),
# and restartUnits/reloadUnits for systemd service integration.
{ config, lib, pkgs, ... }:
let
  cfg = config.akeyless;

  # Resolve template content: inline content or read from file at eval time.
  effectiveTemplateContent = tmpl:
    if tmpl.file != null
    then builtins.readFile tmpl.file
    else tmpl.content;

  manifestFile = pkgs.writeText "akeyless-manifest.json" (builtins.toJSON {
    secrets = lib.mapAttrsToList (name: secret: {
      akeyless_path = name;
      file_path = secret.path;
      mode = secret.mode;
      owner = secret.owner;
      group = secret.group;
      uid = secret.uid;
      gid = secret.gid;
      restart_units = secret.restartUnits;
      reload_units = secret.reloadUnits;
    }) cfg.secrets;

    templates = lib.mapAttrsToList (name: tmpl: {
      inherit name;
      content = effectiveTemplateContent tmpl;
      file_path = tmpl.path;
      mode = tmpl.mode;
      owner = tmpl.owner;
      group = tmpl.group;
      uid = tmpl.uid;
      gid = tmpl.gid;
    }) cfg.templates;

    generations_dir = "/run/akeyless-nix.d";
    symlink_path = "/run/akeyless-nix";
    keep_generations = cfg.keepGenerations;
  });

  # Collect all units that should be restarted when any secret changes.
  allRestartUnits = lib.unique (
    lib.concatMap (secret: secret.restartUnits)
    (lib.attrValues cfg.secrets)
  );

  # Collect all units that should be reloaded when any secret changes.
  allReloadUnits = lib.unique (
    lib.concatMap (secret: secret.reloadUnits)
    (lib.attrValues cfg.secrets)
  );

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
            description = "If true, decrypt before user creation (for password hashes)";
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
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          path = lib.mkOption { type = lib.types.str; description = "Target file path."; };
          content = lib.mkOption {
            type = lib.types.str;
            default = "";
            description = ''
              Template content with placeholder substitution.
              Mutually exclusive with `file` — if both are set, `file` takes precedence.
            '';
          };
          file = lib.mkOption {
            type = lib.types.nullOr lib.types.path;
            default = null;
            description = ''
              Template file (alternative to inline content).
              The file is read at Nix evaluation time and its contents used as the template.
              Takes precedence over `content` if both are set.
            '';
          };
          mode = lib.mkOption { type = lib.types.str; default = "0400"; description = "File permission mode."; };
          owner = lib.mkOption { type = lib.types.str; default = "root"; description = "File owner name."; };
          group = lib.mkOption { type = lib.types.str; default = "root"; description = "File group name."; };
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

    ignorePasswd = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Pass --ignore-passwd to skip owner/group lookups (useful in CI/dry-run).";
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
      ${cfg.package}/bin/akeyless-install-secrets install \
        ${lib.optionalString cfg.ignorePasswd "--ignore-passwd "}\
        ${manifestFile} || \
        echo "akeyless-nix: WARNING — secret installation failed"

      # Restart units that depend on changed secrets.
      # The binary writes secrets atomically; we trigger restarts unconditionally
      # on activation since we cannot yet diff generations from the activation script.
      # Future: the binary could output changed secret names to enable selective restarts.
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
