{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.deadlock-api-ingest;
  # The only place the service writes: dirs::data_dir() for the service user, under the $HOME
  # systemd sets from their account. Users not declared in users.users fall back to /home.
  home = config.users.users.${cfg.user}.home or "/home/${cfg.user}";
  dataDir = "${home}/.local/share/deadlock-api-ingest";
in {
  options.services.deadlock-api-ingest = {
    enable = mkEnableOption "Deadlock API Ingest service";

    package = mkOption {
      type = types.package;
      defaultText = literalExpression "deadlock-api-ingest.packages.\${system}.default";
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

    readGameMemory = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Read match salts from Deadlock's memory while it runs. Grants the service
        CAP_SYS_PTRACE, which would let it read any process's memory on the host; the
        service only ever opens Deadlock's /proc/<pid>/mem, read-only.
      '';
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
        ExecStart = "${cfg.package}/bin/deadlock-api-ingest" + optionalString (!cfg.readGameMemory) " --no-memory";
        # A fresh user has no data directory yet; runuser creates it owned by the user, not root.
        ExecStartPre = "+${pkgs.util-linux}/bin/runuser -u ${cfg.user} -- ${pkgs.coreutils}/bin/mkdir -p ${dataDir}";
        Restart = "on-failure";
        RestartSec = "10s";

        # CAP_SYS_PTRACE opens Deadlock's /proc/<pid>/mem (see readGameMemory); no other
        # capability is kept, and the ptrace syscall itself, which the reader never needs, is denied.
        AmbientCapabilities = optional cfg.readGameMemory "CAP_SYS_PTRACE";
        CapabilityBoundingSet = if cfg.readGameMemory then [ "CAP_SYS_PTRACE" ] else [ "" ];
        SystemCallFilter = [ "~ptrace" ];

        # Hardening (relaxed to allow Steam directory access)
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        # Allow reading home directory for Steam cache
        ProtectHome = "read-only";
        # Steam's directory only needs reading, which ProtectHome allows. "-" skips the data
        # directory while it doesn't exist: without it every command of the unit, ExecStartPre
        # too, fails with 226/NAMESPACE before the directory can be created.
        ReadWritePaths = [ "-${dataDir}" ];
      };
    };
  };
}
