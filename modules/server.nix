{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.nix-local-cache-server;
in {
  options.services.nix-local-cache-server = {
    enable = lib.mkEnableOption "Nix Local Cache Server";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.nix-local-cache-server; # This assumes you add the overlay or package to pkgs
      description = "The server package to use.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 3000;
      description = "Port to listen on.";
    };

    workerThreads = lib.mkOption {
      type = lib.types.int;
      default = 2;
      description = "Number of concurrent build jobs.";
    };

    builders = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "ssh://user@host ...";
      description = "Nix builders configuration string.";
    };

    secretKeyFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "Path to the secret key file for signing the cache.";
    };

    workingDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/nix-local-cache";
      description = "Directory for database, logs, and cache.";
    };

    enableNginx = lib.mkEnableOption "Nginx reverse proxy and static file serving";

    domain = lib.mkOption {
      type = lib.types.str;
      default = "localhost";
      description = "Domain name for Nginx.";
    };
  };

  config = lib.mkIf cfg.enable {
    # Create user and group
    users.users.nix-local-cache = {
      isSystemUser = true;
      group = "nix-local-cache";
      home = cfg.workingDir;
      createHome = false;
      extraGroups = ["nix-users"]; # Needs access to nix-daemon
    };
    users.groups.nix-local-cache = {};

    # Ensure directories exist with correct permissions
    systemd.tmpfiles.rules = [
      "d '${cfg.workingDir}' 0755 nix-local-cache nix-local-cache -"
      "d '${cfg.workingDir}/cache' 0755 nix-local-cache nix-local-cache -"
      "d '${cfg.workingDir}/log' 0755 nix-local-cache nix-local-cache -"
    ];

    # Systemd Service
    systemd.services.nix-local-cache-server = {
      description = "Nix Local Cache Server";
      after = ["network.target"];
      wantedBy = ["multi-user.target"];

      # Environment variables for configuration
      environment =
        {
          NIX_CACHE_PORT = toString cfg.port;
          NIX_CACHE_WORKER_THREADS = toString cfg.workerThreads;
          NIX_CACHE_CACHE_DIR = "${cfg.workingDir}/cache";
          NIX_CACHE_LOG_DIR = "${cfg.workingDir}/log";
          NIX_CACHE_SQLITE_DB_PATH = "${cfg.workingDir}/jobs.sqlite";
          # NIX_CACHE_META_FILE = "${cfg.workingDir}/metadata.json"; # Optional if code handles default
        }
        // lib.optionalAttrs (cfg.builders != null) {
          NIX_CACHE_BUILDERS = cfg.builders;
        }
        // lib.optionalAttrs (cfg.secretKeyFile != null) {
          NIX_CACHE_SECRET_KEY_FILE = cfg.secretKeyFile;
        };

      serviceConfig = {
        User = "nix-local-cache";
        Group = "nix-local-cache";
        ExecStart = "${cfg.package}/bin/nix-local-cache-server";
        Restart = "always";
        WorkingDirectory = cfg.workingDir;
      };

      path = [pkgs.nix pkgs.git pkgs.openssh]; # Binaries needed by the service
    };

    # Nginx Configuration (Optional)
    services.nginx = lib.mkIf cfg.enableNginx {
      enable = true;
      virtualHosts.${cfg.domain} = {
        locations."/" = {
          proxyPass = "http://127.0.0.1:${toString cfg.port}";
          proxyWebsockets = true;
        };

        # Serve the binary cache files directly for performance
        locations."/nix-cache/" = {
          alias = "${cfg.workingDir}/cache/";
          extraConfig = ''
            autoindex on;
          '';
        };

        # TODO: Serve Frontend Assets
        # locations."/dashboard/" = { ... }
      };
    };
  };
}
