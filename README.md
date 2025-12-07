# Nix Local Cache

A robust, local build service and binary cache for Nix, designed to simplify managing builds across your infrastructure. It allows you to queue builds, distribute them to remote builders, and cache the results for immediate use by your fleet.

## Features

*   **Job Queueing:** Submit build requests via API or UI.
*   **Distributed Builds:** Configure remote builders (via SSH) to offload compilation.
*   **Concurrency Control:** Fine-tune how many jobs run in parallel and how many build steps each builder handles.
*   **Web Dashboard:** Monitor build progress, view real-time logs, and cancel running jobs.
*   **Binary Cache:** Serves built artifacts as a standard Nix binary cache (signed).
*   **Metrics:** Prometheus endpoint for monitoring system health.

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
