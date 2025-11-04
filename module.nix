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
      default = "deadlock-api-ingest";
      description = "User account under which deadlock-api-ingest runs";
    };

    group = mkOption {
      type = types.str;
      default = "deadlock-api-ingest";
      description = "Group under which deadlock-api-ingest runs";
    };
  };

  config = mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      description = "Deadlock API Ingest service user";
    };

    users.groups.${cfg.group} = {};

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

        # Hardening
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/var/lib/deadlock-api-ingest" ];
      };
    };

    # Create data directory if needed
    systemd.tmpfiles.rules = [
      "d /var/lib/deadlock-api-ingest 0750 ${cfg.user} ${cfg.group} -"
    ];
  };
}
