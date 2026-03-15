# Home-manager module for akeyless-nix secret management.
#
# Drop-in replacement for sops-nix: declares secrets and templates,
# generates a JSON manifest at eval time, and runs akeyless-install-secrets
# at activation time to fetch and write secret files.
#
# Usage:
#   akeyless.enable = true;
#   akeyless.secrets."/pleme/github/token" = {
#     path = "${homeDir}/.config/github/token";
#     mode = "0600";
#   };
#   akeyless.templates."kubeconfig" = {
#     path = "${homeDir}/.kube/credentials";
#     content = ''
#       token: ${config.akeyless.placeholder."/pleme/k8s/token"}
#     '';
#   };
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.akeyless;

  # Resolve the effective path for a secret: explicit path or auto-generated default.
  effectiveSecretPath = name: secret:
    if secret.path != ""
    then secret.path
    else "${cfg.defaultSymlinkPath}/${lib.replaceStrings ["/"] ["-"] (lib.removePrefix "/" name)}";

  # Resolve template content: inline content or read from file at eval time.
  effectiveTemplateContent = tmpl:
    if tmpl.file != null
    then builtins.readFile tmpl.file
    else tmpl.content;

  # Build the JSON manifest consumed by akeyless-install-secrets at activation time.
  # This is a derivation in the Nix store -- evaluated at build time, secrets fetched
  # at activation time.
  manifestFile = pkgs.writeText "akeyless-manifest.json" (builtins.toJSON {
    secrets =
      lib.mapAttrsToList (name: secret: {
        akeyless_path = name;
        file_path = effectiveSecretPath name secret;
        mode = secret.mode;
        owner = secret.owner;
        group = secret.group;
        uid = secret.uid;
        gid = secret.gid;
      })
      cfg.secrets;

    templates =
      lib.mapAttrsToList (name: tmpl: {
        inherit name;
        content = effectiveTemplateContent tmpl;
        file_path = tmpl.path;
        mode = tmpl.mode;
        owner = tmpl.owner;
        group = tmpl.group;
        uid = tmpl.uid;
        gid = tmpl.gid;
      })
      cfg.templates;

    generations_dir = cfg.defaultSecretsMountPoint;
    symlink_path = cfg.defaultSymlinkPath;
    keep_generations = cfg.keepGenerations;
  });

  secretSubmodule = lib.types.submodule {
    options = {
      path = lib.mkOption {
        type = lib.types.str;
        default = "";
        description = ''
          Local file path to write the secret value.
          If empty, defaults to {defaultSymlinkPath}/{sanitized-name}.
        '';
      };

      mode = lib.mkOption {
        type = lib.types.str;
        default = "0400";
        description = "File permission mode (octal string, e.g. \"0600\").";
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
        description = "File owner UID (alternative to owner name). Takes precedence over owner.";
      };

      gid = lib.mkOption {
        type = lib.types.nullOr lib.types.int;
        default = null;
        description = "File group GID (alternative to group name). Takes precedence over group.";
      };
    };
  };

  templateSubmodule = lib.types.submodule {
    options = {
      path = lib.mkOption {
        type = lib.types.str;
        description = "Local file path to write the rendered template.";
      };

      content = lib.mkOption {
        type = lib.types.str;
        default = "";
        description = ''
          Template content with placeholder substitution.
          Use config.akeyless.placeholder."<secret-path>" to reference secrets.
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

      mode = lib.mkOption {
        type = lib.types.str;
        default = "0400";
        description = "File permission mode (octal string).";
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
        description = "File owner UID (alternative to owner name). Takes precedence over owner.";
      };

      gid = lib.mkOption {
        type = lib.types.nullOr lib.types.int;
        default = null;
        description = "File group GID (alternative to group name). Takes precedence over group.";
      };
    };
  };
in {
  options.akeyless = {
    enable = lib.mkEnableOption "Akeyless secret management (replaces sops-nix)";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.akeyless-install-secrets;
      description = "The akeyless-install-secrets package to use.";
    };

    defaultSecretsMountPoint = lib.mkOption {
      type = lib.types.str;
      default = "${config.xdg.dataHome}/akeyless-nix/generations";
      description = "Directory for secret generations (where generation directories are created).";
    };

    defaultSymlinkPath = lib.mkOption {
      type = lib.types.str;
      default = "${config.xdg.dataHome}/akeyless-nix/secrets";
      description = "Symlink path for current generation. Also used as the base for auto-generated secret paths.";
    };

    secrets = lib.mkOption {
      type = lib.types.attrsOf secretSubmodule;
      default = {};
      description = ''
        Secrets to fetch from Akeyless. Keys are Akeyless paths
        (e.g., "/pleme/prod/db-password").
        If `path` is not set, defaults to `''${defaultSymlinkPath}/{sanitized-name}`.
      '';
      example = lib.literalExpression ''
        {
          "/pleme/github/token" = {
            path = "''${homeDir}/.config/github/token";
            mode = "0600";
          };
        }
      '';
    };

    templates = lib.mkOption {
      type = lib.types.attrsOf templateSubmodule;
      default = {};
      description = ''
        Templates with secret placeholder substitution. Reference secrets
        via config.akeyless.placeholder."<secret-path>".
        Template content can be specified inline via `content` or loaded from
        a file via `file` (read at Nix evaluation time).
      '';
      example = lib.literalExpression ''
        {
          "kubeconfig" = {
            path = "''${homeDir}/.kube/credentials";
            content = "token: ''${config.akeyless.placeholder."/pleme/k8s/token"}";
            mode = "0600";
          };
        }
      '';
    };

    placeholder = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      readOnly = true;
      default = {};
      description = ''
        Auto-generated placeholders for use in templates.
        Each entry maps an Akeyless secret path to a unique placeholder string
        that akeyless-install-secrets replaces with the real value at activation time.
      '';
    };

    keepGenerations = lib.mkOption {
      type = lib.types.int;
      default = 2;
      description = "Number of secret generations to keep.";
    };

    ignorePasswd = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Pass --ignore-passwd to skip owner/group lookups (useful in CI).";
    };
  };

  config = lib.mkIf cfg.enable {
    # Generate placeholders for template substitution.
    # Each placeholder encodes a SHA-256 hash of the secret name so the binary
    # can locate and replace them without ambiguity.
    akeyless.placeholder =
      lib.mapAttrs
      (name: _: "<AKEYLESS:${builtins.hashString "sha256" name}:PLACEHOLDER>")
      cfg.secrets;

    # Ensure XDG data directory exists for generations
    xdg.enable = true;

    # Activation script: run akeyless-install-secrets at rebuild/switch time.
    # Ordered after writeBoundary so home.file targets are in place first.
    # Failure is non-fatal (logged warning) to avoid bricking rebuilds when offline.
    home.activation.akeylessSecrets = lib.hm.dag.entryAfter ["writeBoundary"] ''
      ${cfg.package}/bin/akeyless-install-secrets install \
        ${lib.optionalString cfg.ignorePasswd "--ignore-passwd "}\
        ${manifestFile} \
        || echo "akeyless-nix: WARNING - secret installation failed (offline?)"
    '';
  };
}
