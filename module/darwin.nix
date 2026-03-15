# Darwin module for akeyless-nix.
#
# Imports the home-manager module and adds a launchd agent for boot-time
# secret installation. Manifest copied to stable path for launchd reference.
{ config, lib, pkgs, ... }:
let
  cfg = config.akeyless;
  alib = import ./lib.nix { inherit lib; };
  homeDir = config.home.homeDirectory;

  stableManifestPath = "${config.xdg.dataHome}/akeyless-nix/manifest.json";
  logDir = "${homeDir}/Library/Logs/AkeylessNix";

  manifestFile = alib.buildManifest {
    inherit pkgs cfg;
    generationsDir = cfg.defaultSecretsMountPoint;
    symlinkPath = cfg.defaultSymlinkPath;
  };
in {
  imports = [ ./home-manager.nix ];

  config = lib.mkIf cfg.enable {
    home.activation.akeylessManifestCopy = lib.hm.dag.entryBefore [ "akeylessSecrets" ] ''
      mkdir -p "$(dirname "${stableManifestPath}")"
      cp -f ${manifestFile} "${stableManifestPath}"
    '';

    home.activation.akeylessLogDir = lib.hm.dag.entryBefore [ "akeylessSecrets" ] ''
      mkdir -p "${logDir}"
    '';

    launchd.agents.akeyless-nix = {
      enable = true;
      config = {
        Label = "io.pleme.akeyless-nix";
        ProgramArguments = [
          "${cfg.package}/bin/akeyless-install-secrets"
          "install"
        ] ++ lib.optionals cfg.ignorePasswd [ "--ignore-passwd" ]
          ++ [ stableManifestPath ];
        RunAtLoad = true;
        KeepAlive = false;
        StandardOutPath = "${logDir}/stdout.log";
        StandardErrorPath = "${logDir}/stderr.log";
      };
    };
  };
}
