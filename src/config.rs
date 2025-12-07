use clap::Parser;
use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    pub flake_path: String,
    pub cache_dir: String,
    pub log_dir: String,
    pub meta_file: String,
    pub port: u16,
    pub hosts: Option<Vec<String>>,
    pub worker_threads: usize,
    pub retry_count: u32,
    pub retry_delay_secs: u64,
    pub arch_cores: HashMap<String, u32>,
    pub secret_key_file: Option<String>,
    pub sqlite_db_path: String,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to configuration file
    #[arg(short, long, default_value = "config.toml")]
    pub config: String,

    /// Flake path
    #[arg(long)]
    pub flake_path: Option<String>,

    /// Cache directory
    #[arg(long)]
    pub cache_dir: Option<String>,

    /// Log directory
    #[arg(long)]
    pub log_dir: Option<String>,

    /// Specific hosts to build (comma separated)
    #[arg(long, value_delimiter = ',')]
    pub hosts: Option<Vec<String>>,

    /// Path to the secret key file for signing the cache
    #[arg(long)]
    pub secret_key_file: Option<String>,

    /// Path to the persistent job database file
    #[arg(long)]
    pub sqlite_db_path: Option<String>,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let args = Args::parse();

        let mut builder = Config::builder()
            .set_default("port", 3000)?
            .set_default("worker_threads", 2)?
            .set_default("flake_path", "/home/ikovalev/projects/dotfiles")?
            .set_default("cache_dir", "/mnt/zfs-pool0/nix-cache/cache")?
            .set_default("log_dir", "/mnt/zfs-pool0/nix-local-cache/log")?
            .set_default("meta_file", "/mnt/zfs-pool0/nix-cache/metadata.json")?
            .set_default("retry_count", 3)?
            .set_default("retry_delay_secs", 10)?
            .set_default("arch_cores.aarch64-linux", 1)?
            .set_default("sqlite_db_path", "jobs.sqlite")?
            .add_source(File::with_name(&args.config).required(false))
            .add_source(Environment::with_prefix("NIX_CACHE"));

        // CLI Overrides
        if let Some(v) = args.flake_path {
            builder = builder.set_override("flake_path", v)?;
        }
        if let Some(v) = args.cache_dir {
            builder = builder.set_override("cache_dir", v)?;
        }
        if let Some(v) = args.log_dir {
            builder = builder.set_override("log_dir", v)?;
        }
        if let Some(v) = args.hosts {
            builder = builder.set_override("hosts", v)?;
        }
        if let Some(v) = args.secret_key_file {
            builder = builder.set_override("secret_key_file", v)?;
        }
        if let Some(v) = args.sqlite_db_path {
            builder = builder.set_override("sqlite_db_path", v)?;
        }

        let mut settings: Self = builder.build()?.try_deserialize()?;

        // Canonicalize paths to absolute
        if let Ok(abs_path) = std::fs::canonicalize(&settings.cache_dir) {
            settings.cache_dir = abs_path.to_string_lossy().to_string();
        } else if let Ok(abs_path) = std::path::Path::new(&settings.cache_dir).canonicalize() {
            settings.cache_dir = abs_path.to_string_lossy().to_string();
        } else {
            // Fallback manually if canonicalize fails (e.g. dir doesn't exist yet)
            if !std::path::Path::new(&settings.cache_dir).is_absolute() {
                if let Ok(cwd) = std::env::current_dir() {
                    settings.cache_dir =
                        cwd.join(&settings.cache_dir).to_string_lossy().to_string();
                }
            }
        }

        if !std::path::Path::new(&settings.sqlite_db_path).is_absolute() {
            if let Ok(abs_path) = std::path::Path::new(&settings.sqlite_db_path).canonicalize() {
                settings.sqlite_db_path = abs_path.to_string_lossy().to_string();
            }
        } // Default db_file relative to cache_dir if not set explicitly (or if it's just a filename)
          // Note: Config crate defaults are set before deserialization.
          // But we can't easily refer to "cache_dir" in set_default for another field.
          // So we handle the default logic here if it's missing or empty (though String defaults to empty).
          // Actually, let's just set a dummy default and fix it here.
          // Wait, `try_deserialize` requires all fields.
          // Let's set a default of "jobs.json" in the builder? No, we need the absolute path of cache_dir.
          // Let's make db_file optional in struct? No, user wants persistence.

        // Best approach: Set default to "jobs.json". If it's relative, join with cache_dir.
        // But `db_file` might not be in `builder` if we don't set it.

        // Let's check if `settings.db_file` is empty or relative.
        // Actually, I missed setting a default in the builder above.

        if !std::path::Path::new(&settings.log_dir).is_absolute() {
            if let Ok(cwd) = std::env::current_dir() {
                settings.log_dir = cwd.join(&settings.log_dir).to_string_lossy().to_string();
            }
        }

        if let Some(ref key_file) = settings.secret_key_file {
            if let Ok(abs_path) = std::fs::canonicalize(key_file) {
                settings.secret_key_file = Some(abs_path.to_string_lossy().to_string());
            } else if !std::path::Path::new(key_file).is_absolute() {
                if let Ok(cwd) = std::env::current_dir() {
                    settings.secret_key_file =
                        Some(cwd.join(key_file).to_string_lossy().to_string());
                }
            }
        }

        // Handle db_file logic.
        // If we didn't set it in builder, it might be missing if not in struct default?
        // Since I added it to struct, deserialization will fail if not present.
        // So I MUST set a default in builder.

        Ok(settings)
    }
}
