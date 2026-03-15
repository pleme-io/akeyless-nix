# Shared definitions for akeyless-nix modules.
#
# Extracted to avoid duplication across home-manager, darwin, and NixOS modules.
# Contains: submodule types, manifest builder, helper functions.
{ lib }:

rec {
  # ── Submodule: secret declaration ──────────────────────────────────────
  secretSubmodule = lib.types.submodule {
    options = {
      path = lib.mkOption {
        type = lib.types.str;
        default = "";
        description = "Local file path (empty = auto-generated from symlink path).";
      };
      mode = lib.mkOption {
        type = lib.types.str;
        default = "0400";
        description = "File permission mode (octal).";
      };
      owner = lib.mkOption {
        type = lib.types.str;
        default = "";
        description = "File owner (empty = current user).";
      };
      group = lib.mkOption {
        type = lib.types.str;
        default = "";
        description = "File group (empty = current group).";
      };
      uid = lib.mkOption {
        type = lib.types.nullOr lib.types.int;
        default = null;
        description = "File owner UID (takes precedence over owner).";
      };
      gid = lib.mkOption {
        type = lib.types.nullOr lib.types.int;
        default = null;
        description = "File group GID (takes precedence over group).";
      };
      neededForUsers = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Decrypt before user creation (NixOS only).";
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
  };

  # ── Submodule: template declaration ────────────────────────────────────
  templateSubmodule = lib.types.submodule {
    options = {
      path = lib.mkOption {
        type = lib.types.str;
        description = "Local file path for rendered template.";
      };
      content = lib.mkOption {
        type = lib.types.str;
        default = "";
        description = "Template content (use akeyless.placeholder for substitution).";
      };
      file = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Template file (read at eval time, takes precedence over content).";
      };
      mode = lib.mkOption {
        type = lib.types.str;
        default = "0400";
        description = "File permission mode (octal).";
      };
      owner = lib.mkOption { type = lib.types.str; default = ""; };
      group = lib.mkOption { type = lib.types.str; default = ""; };
      uid = lib.mkOption { type = lib.types.nullOr lib.types.int; default = null; };
      gid = lib.mkOption { type = lib.types.nullOr lib.types.int; default = null; };
    };
  };

  # ── Helpers ────────────────────────────────────────────────────────────

  # Resolve effective secret path (explicit or auto-generated).
  effectiveSecretPath = symlinkPath: name: secret:
    if secret.path != ""
    then secret.path
    else "${symlinkPath}/${lib.replaceStrings ["/"] ["-"] (lib.removePrefix "/" name)}";

  # Resolve template content (inline or from file).
  effectiveTemplateContent = tmpl:
    if tmpl.file != null then builtins.readFile tmpl.file else tmpl.content;

  # Build the JSON manifest from config.
  buildManifest = { pkgs, cfg, generationsDir, symlinkPath }:
    pkgs.writeText "akeyless-manifest.json" (builtins.toJSON {
      secrets = lib.mapAttrsToList (name: secret: {
        akeyless_path = name;
        file_path = effectiveSecretPath symlinkPath name secret;
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

      generations_dir = generationsDir;
      symlink_path = symlinkPath;
      keep_generations = cfg.keepGenerations;
    });

  # Generate placeholder map for template substitution.
  mkPlaceholders = secrets:
    lib.mapAttrs (name: _:
      "<AKEYLESS:${builtins.hashString "sha256" name}:PLACEHOLDER>"
    ) secrets;

  # Collect all restart/reload units from secrets.
  collectRestartUnits = secrets:
    lib.unique (lib.concatMap (s: s.restartUnits) (lib.attrValues secrets));
  collectReloadUnits = secrets:
    lib.unique (lib.concatMap (s: s.reloadUnits) (lib.attrValues secrets));
}
