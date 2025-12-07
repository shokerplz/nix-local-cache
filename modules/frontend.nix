{ config, lib, pkgs, ... }:

let
  cfg = config.services.nix-local-cache-frontend;
in
{
  options.services.nix-local-cache-frontend = {
    enable = lib.mkEnableOption "Nix Local Cache Frontend";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.nix-local-cache-frontend; # Assumes this is available in pkgs (via overlay or flake)
      description = "The frontend package to use.";
    };

    domain = lib.mkOption {
      type = lib.types.str;
      default = "localhost";
      description = "Domain name for the Nginx virtual host.";
    };

    apiUrl = lib.mkOption {
      type = lib.types.str;
      default = "http://localhost:3000";
      description = "URL of the Nix Local Cache API.";
    };
  };

  config = lib.mkIf cfg.enable {
    services.nginx = {
      enable = true;
      virtualHosts.${cfg.domain} = {
        root = "${cfg.package}";
        locations."/" = {
          tryFiles = "$uri $uri/ /index.html";
        };
        # Serve the config.js dynamically or override the file
        locations."/config.js" = {
          alias = pkgs.writeText "config.js" ''
            window.SERVER_CONFIG = {
              apiUrl: "${cfg.apiUrl}"
            };
          '';
        };
      };
    };
  };
}
