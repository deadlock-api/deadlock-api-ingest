# nix-darwin module: runs the watcher as a launch agent of `system.primaryUser`, the user
# whose Steam it reads. Memory reading and self-update don't exist on macOS.
{ config, lib, ... }:

let
  cfg = config.services.deadlock-api-ingest;
in
{
  options.services.deadlock-api-ingest = {
    enable = lib.mkEnableOption "Deadlock API Ingest service";

    package = lib.mkOption {
      type = lib.types.package;
      defaultText = lib.literalExpression "deadlock-api-ingest.packages.\${system}.default";
      description = "The deadlock-api-ingest package to use";
    };

    gc = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Ask Steam's Game Coordinator for missing match salts while Deadlock is closed,
        using the saved Steam sessions on this Mac (see the README).
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    # `deadlock-api-ingest --gui` and `--own-matches`, and the status window app.
    environment.systemPackages = [ cfg.package ];

    launchd.user.agents.deadlock-api-ingest = {
      serviceConfig = {
        # The label install-macos.sh and the status window use, so the window finds this agent.
        Label = "com.deadlock-api.deadlock-api-ingest";
        ProgramArguments = [
          "${cfg.package}/bin/deadlock-api-ingest"
        ] ++ lib.optional (!cfg.gc) "--no-gc";
        RunAtLoad = true;
        # Restarted when it fails, not after a clean exit (e.g. when another copy already runs).
        KeepAlive.SuccessfulExit = false;
        ThrottleInterval = 10;
      };
      managedBy = "services.deadlock-api-ingest.enable";
    };
  };
}
