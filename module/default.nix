# Re-export the home-manager module as the default.
#
# Consumers that only need the home-manager module (works on both Darwin
# and Linux) import this. For Darwin-specific launchd integration, import
# module/darwin.nix instead.
import ./home-manager.nix
