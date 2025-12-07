CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY NOT NULL,
    hosts TEXT NOT NULL,
    status TEXT NOT NULL,
    status_message TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    log_path TEXT NOT NULL,
    flake_ref TEXT NOT NULL
)
