# Nix Local Cache

A robust, local build service and binary cache for Nix, designed to simplify managing builds across your infrastructure. It allows you to queue builds, distribute them to remote builders, and cache the results for immediate use by your fleet.

## Features

*   **Job Queueing:** Submit build requests via API or UI.
*   **Distributed Builds:** Configure remote builders (via SSH) to offload compilation.
*   **Concurrency Control:** Fine-tune how many jobs run in parallel and how many build steps each builder handles.
*   **Web Dashboard:** Monitor build progress, view real-time logs, and cancel running jobs.
*   **Binary Cache:** Populates a directory with a standard Nix binary cache structure (signed). You need a web server (like Nginx) to serve this directory.
*   **Metrics:** Prometheus endpoint for monitoring system health.

## Deployment Hints & Caveats

### Serving the Binary Cache

The `nix-local-cache-server` populates a directory (specified by `workingDir` / `cache`) with signed NARs and `.narinfo` files. To make this usable by other machines, you must serve this directory via HTTP.

**Nginx Example:**
```nginx
server {
    listen 80;
    server_name cache.local;
    root /path/to/nix-cache/cache;
    
    location / {
        autoindex on;
    }
}
```

### Private Repositories

If your flake is in a private Git repository, the server needs SSH access.
1.  **SSH Key:** Ensure the user running the service has an SSH key authorized to access the repo.
2.  **GIT_SSH_COMMAND:** Set this environment variable to specify the key if it's not the default.
3.  **Known Hosts:** The system user (e.g., `nix-local-cache`) must have the git server's host key in its `known_hosts`, or you must configure global known hosts.

**Systemd Example:**
```nix
systemd.services.nix-local-cache-server = {
  environment.GIT_SSH_COMMAND = "ssh -i /path/to/private/key -o IdentitiesOnly=yes";
};

programs.ssh.knownHosts = {
  "github.com" = { publicKey = "..."; };
  "git.your-domain.com" = { publicKey = "..."; };
};
```

### Frontend Configuration in Production

If you deploy the frontend separately (e.g., via Nginx or a CDN) or behind a reverse proxy, you need to tell it where the API is. The frontend looks for a global `window.SERVER_CONFIG` object.

You can inject this by serving a `config.js` file at the root:

**Nginx Example:**
```nix
location = /config.js {
    alias = pkgs.writeText "config.js" ''
        window.SERVER_CONFIG = {
            apiUrl: "https://api.builder.example.com"
        };
    '';
}
```

## Getting Started

### Prerequisites

*   **Nix:** with `direnv` (recommended) or `devenv`.
*   **Rust:** (managed via `devenv`).
*   **Bun:** (managed via `devenv`).

### Development Environment

Enter the development shell:

```bash
devenv shell
```

### Running the Service

You can run the backend service using `cargo`:

```bash
cargo run -p nix-local-cache-server -- \
  --worker-threads 4 \
  --builders "ssh://user@beefy-server x86_64-linux - - 1 8"
```

### Client Usage

The `nix-local-cache-client` binary allows you to list and apply builds on target machines.

### Installation

Build the client:
```bash
cargo build -p nix-local-cache-client --release
# Copy target/release/nix-local-cache-client to your machines
```

### Commands

*   **List builds:**
    ```bash
    nix-local-cache-client list --api http://cache.local:3000
    ```
    Filters builds for the current hostname. Use `--host <name>` to override.

*   **Apply a build:**
    *   **Interactive:**
        ```bash
        nix-local-cache-client apply --api http://cache.local:3000
        ```
        Shows a TUI list of available builds. Select one to download and switch to it.

    *   **Non-interactive:**
        ```bash
        nix-local-cache-client apply <UUID> --yes --api http://cache.local:3000
        ```

## Installation via Nix Flakes

You can consume this project as a Flake input in your NixOS configuration.

**flake.nix:**
```nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    nix-local-cache.url = "git+ssh://gitlab@git.ikovalev.nl/nix/nix-local-cache.git";
  };

  outputs = { self, nixpkgs, nix-local-cache, ... }: {
    nixosConfigurations.my-server = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        nix-local-cache.nixosModules.server
      ];
    };
  };
}
```

**configuration.nix (Server):**
```nix
{ config, pkgs, ... }: {
  services.nix-local-cache-server = {
    enable = true;
    port = 3000;
    workerThreads = 4;
    builders = "ssh://builder@host x86_64-linux - - 1 8";
    enableNginx = true;
    domain = "cache.local";
  };
}
```

**Client Installation:**
Add to `environment.systemPackages`:
```nix
environment.systemPackages = [
  inputs.nix-local-cache.packages.${pkgs.system}.client
];
```

## Development Workflow

1.  **Workspace:**
    The project is organized as a Cargo workspace:
    *   `crates/server`: The backend API and build worker.
    *   `crates/client`: The CLI tool for target machines.
    *   `crates/common`: Shared types.

2.  **Backend:**
    *   Edit code in `crates/server/src/`.
    *   Run `cargo check` to verify workspace.
    *   Run `cargo run -p nix-local-cache-server` to test.

3.  **Client:**
    *   Edit code in `crates/client/src/`.
    *   Run `cargo run -p nix-local-cache-client` to test.

4.  **Frontend:**
    *   Edit code in `frontend/src/`.
    *   Run `bun run dev` for HMR.
    *   Run `bun run build` to compile for production.

5.  **Database:**
    *   Migrations are in `crates/server/migrations/`.
    *   Run `sqlx migrate run --source crates/server/migrations` to apply.
## License

[MIT](LICENSE)
