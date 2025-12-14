use clap::Parser;
use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use std::{collections::HashMap, fs, path};

#[derive(Debug, Deserialize, Clone)]
pub struct Settings {
    pub flake_path: String,
    pub cache_dir: String,
    pub log_dir: String,
    pub port: u16,
    pub hosts: Option<Vec<String>>,
    pub worker_threads: usize,
    pub retry_count: u32,
    pub retry_delay_secs: u64,
    pub arch_cores: HashMap<String, u32>,
    pub secret_key_file: Option<String>,
    pub sqlite_db_path: String,
    pub builders: Option<String>,
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

    /// Nix builders configuration (e.g. "ssh://machine x86_64-linux ...")
    #[arg(long)]
    pub builders: Option<String>,

    /// Number of concurrent worker threads processing jobs
    #[arg(long)]
    pub worker_threads: Option<usize>,
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let args = Args::parse();

        let mut builder = Config::builder()
            .set_default("port", 3000)?
            .set_default("worker_threads", 1)?
            .set_default("flake_path", "/home/ikovalev/projects/dotfiles")?
            .set_default("cache_dir", "/mnt/zfs-pool0/nix-cache/cache")?
            .set_default("log_dir", "/mnt/zfs-pool0/nix-local-cache/log")?
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
        if let Some(v) = args.builders {
            builder = builder.set_override("builders", v)?;
        }
        if let Some(v) = args.worker_threads {
            builder = builder.set_override("worker_threads", v as i64)?;
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
        }

        if !std::path::Path::new(&settings.log_dir).is_absolute() {
            if let Ok(cwd) = std::env::current_dir() {
                settings.log_dir = cwd.join(&settings.log_dir).to_string_lossy().to_string();
            }
        }

        // It's better not to canonicalize secret file path, because then we need to restart every
        // time secret path changes. Here we just want to verify that this path exist
        if let Some(ref key_file) = settings.secret_key_file {
            if let Ok(exists) = fs::exists(key_file) {
                if exists {
                    if std::path::Path::new(key_file).is_relative() {
                        if let Ok(absolute_path) = std::path::absolute(key_file) {
                            settings.secret_key_file =
                                Some(absolute_path.to_string_lossy().to_string());
                        }
                    }
                } else {
                    return Err(ConfigError::NotFound(format!(
                        "secret file {} does not exist",
                        key_file
                    )));
                }
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

        Ok(settings)
    }
}
