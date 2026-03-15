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

  # Build the manifest (same as home-manager.nix -- shared via import)
  manifestFile = pkgs.writeText "akeyless-manifest.json" (builtins.toJSON {
    secrets =
      lib.mapAttrsToList (name: secret: {
        akeyless_path = name;
        file_path = secret.path;
        mode = secret.mode;
        owner = secret.owner;
        group = secret.group;
      })
      cfg.secrets;

    templates =
      lib.mapAttrsToList (name: tmpl: {
        inherit name;
        content = tmpl.content;
        file_path = tmpl.path;
        mode = tmpl.mode;
        owner = tmpl.owner;
        group = tmpl.group;
      })
      cfg.templates;

    generations_dir = "${config.xdg.dataHome}/akeyless-nix/generations";
    symlink_path = "${config.xdg.dataHome}/akeyless-nix/secrets";
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
