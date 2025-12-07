{
  pkgs,
  lib,
  config,
  inputs,
  ...
}: {
  env.DATABASE_URL = "sqlite://./db.sqlite3";
  packages = [
    pkgs.git
    pkgs.openssl
    pkgs.pkg-config
    pkgs.bun
    pkgs.sqlx-cli
    pkgs.sqlite
  ];
  languages.rust = {
    enable = true;
    channel = "stable";
    version = "1.90.0";
  };
  
  # Optional: Add helper scripts if needed, but cargo handles workspace fine.
}