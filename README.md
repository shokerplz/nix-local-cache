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
cargo run -- \
  --worker-threads 4 \
  --builders "ssh://user@beefy-server x86_64-linux - - 1 8"
```

*   `--worker-threads`: How many jobs the service processes from its queue concurrently.
*   `--builders`: Standard Nix builder configuration string.

### Running the Frontend

In a separate terminal (inside `devenv shell`):

```bash
cd frontend
bun run dev
```

Visit `http://localhost:5173` (or the port Vite displays) to access the dashboard.

## Configuration

Configuration is handled via `config.toml`, environment variables (`NIX_CACHE_*`), or CLI arguments.

**Key Options:**

| Option | CLI Arg | Env Var |
| :--- | :--- | :--- |
| **Worker Threads** | `--worker-threads` | `NIX_CACHE_WORKER_THREADS` |
| **Builders** | `--builders` | `NIX_CACHE_BUILDERS` |
| **Port** | (none, config only) | `NIX_CACHE_PORT` |
| **Cache Dir** | `--cache-dir` | `NIX_CACHE_CACHE_DIR` |
| **Log Dir** | `--log-dir` | `NIX_CACHE_LOG_DIR` |

## Client Usage

The `nix-local-cache-client` binary allows you to list and apply builds on target machines.

### Installation

Build the client:
```bash
cargo build --bin client --release
# Copy target/release/client to your machines (e.g. as nix-local-cache-client)
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

*   **Configuration:**
    *   `NIX_LOCAL_CACHE_API`: Server API URL (default: `http://localhost:3000`).
    *   `NIX_LOCAL_CACHE_URI`: Binary cache URL (default: same as API). Use this if your binary cache is served on a different port/path/domain (e.g. Nginx).

## API Usage

*   **List Jobs:** `GET /jobs`
*   **Get Job:** `GET /jobs/:id`
*   **Cancel Job:** `POST /jobs/:id/cancel`
*   **Trigger Build:** `POST /build`
    ```json
    {
      "flake_url": "github:owner/repo",
      "flake_branch": "main",
      "hosts": ["host-a", "host-b"]
    }
    ```

## Development Workflow

1.  **Backend:**
    *   Edit code in `src/`.
    *   Run `cargo check` to verify.
    *   Run `cargo run` to test.

2.  **Frontend:**
    *   Edit code in `frontend/src/`.
    *   Run `bun run dev` for HMR.
    *   Run `bun run build` to compile for production.

3.  **Database:**
    *   Migrations are in `migrations/`.
    *   Run `sqlx migrate run` to apply.

## License

[MIT](LICENSE)
