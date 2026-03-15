# Darwin-specific module for akeyless-nix.
#
# Imports the home-manager module and adds a launchd agent for boot-time
# secret installation. On macOS, the activation script runs at rebuild time,
# but this agent ensures secrets are also refreshed on reboot or login.
#
# The manifest path is written to a stable location so the launchd agent
# can reference it without depending on Nix store paths changing.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.akeyless;
  homeDir = config.home.homeDirectory;

  # Write the manifest to a stable path that launchd can reference.
  # The activation script copies the Nix store manifest here.
  stableManifestPath = "${config.xdg.dataHome}/akeyless-nix/manifest.json";

  # Resolve template content: inline content or read from file at eval time.
  effectiveTemplateContent = tmpl:
    if tmpl.file != null
    then builtins.readFile tmpl.file
    else tmpl.content;

  # Resolve the effective path for a secret: explicit path or auto-generated default.
  effectiveSecretPath = name: secret:
    if secret.path != ""
    then secret.path
    else "${cfg.defaultSymlinkPath}/${lib.replaceStrings ["/"] ["-"] (lib.removePrefix "/" name)}";

  # Build the manifest (same shape as home-manager.nix)
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

  logDir = "${homeDir}/Library/Logs/AkeylessNix";
in {
  imports = [./home-manager.nix];

  config = lib.mkIf cfg.enable {
    # Copy manifest to a stable path so launchd can reference it.
    # This runs as part of the activation, before the launchd agent starts.
    home.activation.akeylessManifestCopy = lib.hm.dag.entryBefore ["akeylessSecrets"] ''
      mkdir -p "$(dirname "${stableManifestPath}")"
      cp -f ${manifestFile} "${stableManifestPath}"
    '';

    # Ensure log directory exists
    home.activation.akeylessLogDir = lib.hm.dag.entryBefore ["akeylessSecrets"] ''
      mkdir -p "${logDir}"
    '';

    # Launchd agent for boot-time / login-time secret installation.
    # Runs once at load (not kept alive) -- just ensures secrets exist after reboot.
    launchd.agents.akeyless-nix = {
      enable = true;
      config = {
        Label = "io.pleme.akeyless-nix";
        ProgramArguments = [
          "${cfg.package}/bin/akeyless-install-secrets"
          "install"
          stableManifestPath
        ];
        RunAtLoad = true;
        KeepAlive = false;
        StandardOutPath = "${logDir}/stdout.log";
        StandardErrorPath = "${logDir}/stderr.log";
      };
    };
  };
}
