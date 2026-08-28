{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.deadlock-api-ingest;
  
  # Build the package directly in the module
  defaultPackage = pkgs.callPackage ./default.nix { 
    src = ./.;
  };
in {
  options.services.deadlock-api-ingest = {
    enable = mkEnableOption "Deadlock API Ingest service";

    package = mkOption {
      type = types.package;
      default = defaultPackage;
      defaultText = literalExpression "pkgs.callPackage ./default.nix { }";
      description = "The deadlock-api-ingest package to use";
    };

    user = mkOption {
      type = types.str;
      example = "yourusername";
      description = ''
        User account under which deadlock-api-ingest runs.
        This should be the user who has Steam installed.
        Leave unset to create a system user (requires manual Steam directory configuration).
      '';
    };

    group = mkOption {
      type = types.str;
      default = "users";
      description = "Group under which deadlock-api-ingest runs";
    };

    steamUser = mkOption {
      type = types.nullOr types.str;
      default = cfg.user;
      example = "yourusername";
      description = ''
        The user whose Steam directory should be monitored.
        Defaults to the service user. Set this if running as a different user.
      '';
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.user != "deadlock-api-ingest" || cfg.steamUser != null;
        message = "You must set services.deadlock-api-ingest.user to your Steam user, or configure steamUser";
      }
    ];

    users.users = mkIf (cfg.user == "deadlock-api-ingest") {
      deadlock-api-ingest = {
        isSystemUser = true;
        group = cfg.group;
        description = "Deadlock API Ingest service user";
      };
    };

    users.groups = mkIf (cfg.group == "deadlock-api-ingest") {
      deadlock-api-ingest = {};
    };

    systemd.services.deadlock-api-ingest = {
      description = "Deadlock API Ingest Service";
      documentation = [ "https://github.com/deadlock-api/deadlock-api-ingest" ];
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/deadlock-api-ingest";
        Restart = "on-failure";
        RestartSec = "10s";

        # Hardening (relaxed to allow Steam directory access)
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        # Allow reading home directory for Steam cache
        ProtectHome = "read-only";
        # Allow writing to data directory and Steam user's home
        ReadWritePaths = [ 
          "/var/lib/deadlock-api-ingest"
          "/home/${cfg.user}/.local/share/deadlock-api-ingest"
          "/home/${cfg.user}/.local/share/Steam"
        ];
      };
    };

    # Create data directory if needed
    systemd.tmpfiles.rules = [
      "d /var/lib/deadlock-api-ingest 0750 ${cfg.user} ${cfg.group} -"
    ];
  };
}
