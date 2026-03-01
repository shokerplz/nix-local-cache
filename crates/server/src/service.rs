use crate::config::Settings;
use crate::nix;
use anyhow::{Context, Result};
use chrono::Local;
use dashmap::DashMap;
use nix_local_cache_common::{BuildRequest, Job, JobStatus};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};
use uuid::Uuid;

pub struct BuildService {
    pub settings: Settings,
    pub jobs: Arc<DashMap<Uuid, Job>>,
    pub running_jobs: Arc<DashMap<Uuid, tokio::task::JoinHandle<()>>>,
    queue_tx: mpsc::Sender<Uuid>,
    db_pool: SqlitePool,
    nix_ops: Arc<crate::nix::NixOps>,
}

const DEFAULT_JOB_TIMEOUT_SECONDS: u64 = 12 * 60 * 60;
const CANCELLED_BY_USER_MESSAGE: &str = "Cancelled by user";

impl BuildService {
    pub async fn new(
        settings: Settings,
        nix_ops: Arc<crate::nix::NixOps>,
    ) -> Result<(Self, mpsc::Receiver<Uuid>)> {
        let (tx, rx) = mpsc::channel(100);

        // Ensure DB file exists
        if !Path::new(&settings.sqlite_db_path).exists() {
            info!(
                "Database file not found at {}, creating...",
                settings.sqlite_db_path
            );
            File::create(&settings.sqlite_db_path)?;
        }

        let db_pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&format!("sqlite:{}", settings.sqlite_db_path))
            .await?;

        let service = Self {
            settings,
            jobs: Arc::new(DashMap::new()),
            running_jobs: Arc::new(DashMap::new()),
            queue_tx: tx,
            db_pool,
            nix_ops,
        };
        Ok((service, rx))
    }

    pub async fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.settings.cache_dir)?;
        fs::create_dir_all(&self.settings.log_dir)?;

        let cache_info = Path::new(&self.settings.cache_dir).join("nix-cache-info");
        if !cache_info.exists() {
            let mut f = File::create(cache_info)?;
            writeln!(f, "StoreDir: /nix/store")?;
            writeln!(f, "WantMassQuery: 1")?;
            writeln!(f, "Priority: 40")?;
        }

        sqlx::migrate!().run(&self.db_pool).await?;

        self.load_jobs_from_db().await?;

        Ok(())
    }

    pub async fn restart_job(&self, job_id: Uuid) -> Result<()> {
        if let Some(mut job) = self.jobs.get_mut(&job_id) {
            if !matches!(job.status, JobStatus::Failed) {
                return Err(anyhow::anyhow!("Only failed jobs can be restarted"));
            }

            job.status = JobStatus::Queued;
            job.status_message = None;
            job.started_at = None;
            job.finished_at = None;
            job.current_host = None;
            job.results = Some(std::collections::HashMap::new());

            let log_path = Path::new(&self.settings.log_dir).join(&job.log_path);
            File::create(&log_path)?;

            let job_clone = job.clone();
            drop(job);

            self.update_job_in_db(&job_clone).await?;
            self.queue_tx.send(job_id).await?;

            info!("Restarted job {}", job_id);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Job not found"))
        }
    }

    pub async fn cancel_job(&self, job_id: Uuid) -> Result<()> {
        if let Some((_, handle)) = self.running_jobs.remove(&job_id) {
            handle.abort();
        }

        self.mark_job_cancelled(job_id).await?;
        info!("Cancelled job {}", job_id);

        Ok(())
    }

    async fn mark_job_cancelled(&self, job_id: Uuid) -> Result<()> {
        let mut job = self
            .jobs
            .get_mut(&job_id)
            .ok_or_else(|| anyhow::anyhow!("Job not found"))?;

        if matches!(job.status, JobStatus::Completed) {
            return Err(anyhow::anyhow!("Job is already completed"));
        }

        if matches!(job.status, JobStatus::Failed)
            && job.status_message.as_deref() == Some(CANCELLED_BY_USER_MESSAGE)
        {
            return Ok(());
        }

        if matches!(job.status, JobStatus::Failed) {
            return Err(anyhow::anyhow!("Job is already failed"));
        }

        job.status = JobStatus::Failed;
        job.status_message = Some(CANCELLED_BY_USER_MESSAGE.to_string());
        job.finished_at = Some(Local::now());
        job.current_host = None;

        let job_clone = job.clone();
        drop(job);

        self.update_job_in_db(&job_clone).await
    }

    async fn load_jobs_from_db(&self) -> Result<()> {
        let rows = sqlx::query("SELECT * FROM jobs ORDER BY id LIMIT 100")
            .fetch_all(&self.db_pool)
            .await?;

        let mut jobs = Vec::new();
        for row in rows {
            let job = Job {
                id: row.try_get("id")?,
                hosts: serde_json::from_str(row.try_get("hosts")?)?,
                status: row.try_get("status")?,
                status_message: row.try_get("status_message")?,
                created_at: row.try_get("created_at")?,
                started_at: row.try_get("started_at")?,
                finished_at: row.try_get("finished_at")?,
                log_path: row.try_get("log_path")?,
                flake_ref: row.try_get("flake_ref")?,
                timeout_seconds: row
                    .try_get::<i64, _>("timeout_seconds")
                    .ok()
                    .and_then(|value| u64::try_from(value).ok())
                    .unwrap_or(DEFAULT_JOB_TIMEOUT_SECONDS),
                results: row
                    .try_get("results")
                    .unwrap_or_else(|_| None)
                    .and_then(|v: String| serde_json::from_str(&v).ok()),
                current_host: row.try_get("current_host")?,
            };
            jobs.push(job);
        }

        for mut job in jobs {
            if matches!(job.status, JobStatus::Running) {
                job.status = JobStatus::Failed;
                job.status_message = Some("Interrupted by service restart".to_string());
                job.finished_at = Some(Local::now());
                self.update_job_in_db(&job).await?;
            }
            self.jobs.insert(job.id, job.clone());
        }
        info!("Loaded {} jobs from database", self.jobs.len());
        Ok(())
    }

    async fn insert_job_into_db(&self, job: &Job) -> Result<()> {
        let hosts_json = sqlx::types::Json(&job.hosts);
        let status = job.status.clone();
        let results_json = sqlx::types::Json(&job.results);
        let timeout_seconds = i64::try_from(job.timeout_seconds).context("timeout is too large")?;
        sqlx::query(
            r#"
            INSERT INTO jobs (id, hosts, status, status_message, created_at, started_at, finished_at, log_path, flake_ref, timeout_seconds, results, current_host)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(job.id)
        .bind(hosts_json)
        .bind(status)
        .bind(job.status_message.as_deref())
        .bind(job.created_at)
        .bind(job.started_at)
        .bind(job.finished_at)
        .bind(&job.log_path)
        .bind(&job.flake_ref)
        .bind(timeout_seconds)
        .bind(results_json)
        .bind(job.current_host.as_deref())
        .execute(&self.db_pool)
        .await?;
        Ok(())
    }

    async fn update_job_in_db(&self, job: &Job) -> Result<()> {
        let hosts_json = sqlx::types::Json(&job.hosts);
        let status = job.status.clone();
        let results_json = sqlx::types::Json(&job.results);
        let timeout_seconds = i64::try_from(job.timeout_seconds).context("timeout is too large")?;
        sqlx::query(
            r#"
            UPDATE jobs
            SET
                hosts = ?,
                status = ?,
                status_message = ?,
                created_at = ?,
                started_at = ?,
                finished_at = ?,
                log_path = ?,
                flake_ref = ?,
                timeout_seconds = ?,
                results = ?,
                current_host = ?
            WHERE id = ?
            "#,
        )
        .bind(hosts_json)
        .bind(status)
        .bind(job.status_message.as_deref())
        .bind(job.created_at)
        .bind(job.started_at)
        .bind(job.finished_at)
        .bind(&job.log_path)
        .bind(&job.flake_ref)
        .bind(timeout_seconds)
        .bind(results_json)
        .bind(job.current_host.as_deref())
        .bind(job.id)
        .execute(&self.db_pool)
        .await?;
        Ok(())
    }

    pub async fn get_jobs(&self, limit: i64, offset: i64) -> Result<(Vec<Job>, i64)> {
        let total_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs")
            .fetch_one(&self.db_pool)
            .await?;

        let rows = sqlx::query("SELECT * FROM jobs ORDER BY created_at DESC LIMIT ? OFFSET ?")
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.db_pool)
            .await?;

        let mut jobs = Vec::new();
        for row in rows {
            let job = Job {
                id: row.try_get("id")?,
                hosts: serde_json::from_str(row.try_get("hosts")?)?,
                status: row.try_get("status")?,
                status_message: row.try_get("status_message")?,
                created_at: row.try_get("created_at")?,
                started_at: row.try_get("started_at")?,
                finished_at: row.try_get("finished_at")?,
                log_path: row.try_get("log_path")?,
                flake_ref: row.try_get("flake_ref")?,
                timeout_seconds: row
                    .try_get::<i64, _>("timeout_seconds")
                    .ok()
                    .and_then(|value| u64::try_from(value).ok())
                    .unwrap_or(DEFAULT_JOB_TIMEOUT_SECONDS),
                results: row
                    .try_get("results")
                    .unwrap_or_else(|_| None)
                    .and_then(|v: String| serde_json::from_str(&v).ok()),
                current_host: row.try_get("current_host")?,
            };
            jobs.push(job);
        }

        Ok((jobs, total_count))
    }

    pub async fn get_hosts(&self, flake_ref: &str) -> Result<Vec<String>> {
        self.nix_ops.get_hosts(flake_ref).await
    }

    pub async fn submit_build(&self, req: BuildRequest) -> Result<Uuid> {
        let id = Uuid::new_v4();

        let flake_ref =
            nix::resolve_flake_ref(req.flake_url, req.flake_branch, &self.settings.flake_path);

        let target_hosts = if let Some(hosts) = req.hosts {
            hosts
        } else if let Some(hosts) = &self.settings.hosts {
            hosts.clone()
        } else {
            self.nix_ops.get_hosts(&flake_ref).await?
        };

        let timeout_seconds = req
            .timeout_seconds
            .unwrap_or(DEFAULT_JOB_TIMEOUT_SECONDS);
        if timeout_seconds == 0 {
            return Err(anyhow::anyhow!("timeout_seconds must be greater than 0"));
        }

        let log_file_name = format!("{}.log", id);
        let log_path = Path::new(&self.settings.log_dir).join(&log_file_name);
        File::create(&log_path)?;

        let job = Job {
            id,
            hosts: target_hosts,
            status: JobStatus::Queued,
            status_message: None,
            created_at: Local::now(),
            started_at: None,
            finished_at: None,
            log_path: log_file_name,
            flake_ref,
            timeout_seconds,
            results: Some(std::collections::HashMap::new()),
            current_host: None,
        };

        self.jobs.insert(id, job.clone());
        self.insert_job_into_db(&job).await?;
        self.queue_tx.send(id).await?;

        Ok(id)
    }

    pub async fn process_job(&self, job_id: Uuid) {
        if let Some(job) = self.jobs.get(&job_id) {
            if matches!(job.status, JobStatus::Failed)
                && job.status_message.as_deref() == Some(CANCELLED_BY_USER_MESSAGE)
            {
                info!("Skipping cancelled job {}", job_id);
                return;
            }
        }

        let mut job = match self.jobs.get_mut(&job_id) {
            Some(j) => j,
            None => return,
        };

        job.status = JobStatus::Running;
        job.started_at = Some(Local::now());

        let job_data = job.clone();
        drop(job);

        if let Err(e) = self.update_job_in_db(&job_data).await {
            error!("Failed to update job {} in DB: {}", job_id, e);
        }

        let hosts = job_data.hosts.clone();
        let log_path_str = job_data.log_path.clone();
        let flake_ref = job_data.flake_ref.clone();
        let timeout_seconds = job_data.timeout_seconds;

        let log_full_path = Path::new(&self.settings.log_dir).join(&log_path_str);

        info!(
            "Starting job {} for hosts {:?} using flake {}",
            job_id, hosts, flake_ref
        );

        let log_full_path_for_work = log_full_path.clone();
        let flake_ref_for_work = flake_ref.clone();

        let (success, status_message) =
            match tokio::time::timeout(Duration::from_secs(timeout_seconds), async move {
                let mut success = true;
                let mut status_message: Option<String> = None;

                for host in hosts {
                    if let Some(mut job) = self.jobs.get_mut(&job_id) {
                        job.current_host = Some(host.clone());
                        let job_clone = job.clone();
                        drop(job);
                        if let Err(e) = self.update_job_in_db(&job_clone).await {
                            error!("Failed to update job {} current_host: {}", job_id, e);
                        }
                    }

                    if let Err(e) = self
                        .process_host(&host, &flake_ref_for_work, &log_full_path_for_work, job_id)
                        .await
                    {
                        let msg = format!("Job {} failed for host {}: {}", job_id, host, e);
                        error!("{}", msg);
                        status_message = Some(msg);
                        success = false;
                    }
                }

                (success, status_message)
            })
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    let timeout_message =
                        format!("Job timed out after {} seconds", timeout_seconds);
                    error!("Job {} timed out after {} seconds", job_id, timeout_seconds);

                    if let Ok(mut log_file) = tokio::fs::OpenOptions::new()
                        .append(true)
                        .open(&log_full_path)
                        .await
                    {
                        use tokio::io::AsyncWriteExt;
                        let _ = log_file
                            .write_all(
                                format!(
                                    "[{}] {}\n",
                                    Local::now().format("%F %H:%M:%S.%3f"),
                                    timeout_message
                                )
                                .as_bytes(),
                            )
                            .await;
                    }

                    (false, Some(timeout_message))
                }
            };

        let mut job = match self.jobs.get_mut(&job_id) {
            Some(j) => j,
            None => {
                error!("Job {} disappeared from job list while processing!", job_id);
                return;
            }
        };
        job.finished_at = Some(Local::now());
        job.current_host = None;
        job.status = if success {
            JobStatus::Completed
        } else {
            JobStatus::Failed
        };
        job.status_message = status_message;

        info!("Job {} finished with status {:?}", job_id, job.status);

        let job_data = job.clone();
        drop(job);

        if let Err(e) = self.update_job_in_db(&job_data).await {
            error!("Failed to update job {} in DB: {}", job_id, e);
        }
    }

    async fn build_with_retry(
        &self,
        flake_ref: &str,
        host: &str,
        cores: Option<u32>,
        log_file: &mut tokio::fs::File,
    ) -> Result<String> {
        let mut attempts = 0;
        loop {
            let res = self
                .nix_ops
                .build_system(
                    flake_ref,
                    host,
                    cores,
                    self.settings.builders.as_deref(),
                    log_file,
                )
                .await;
            match res {
                Ok(v) => return Ok(v),
                Err(e) => {
                    attempts += 1;
                    if attempts >= self.settings.retry_count {
                        return Err(e);
                    }
                    let delay = self.settings.retry_delay_secs;
                    use tokio::io::AsyncWriteExt;
                    let msg = format!(
                        "Build failed (attempt {}/{}): {}. Retrying in {}s...\n",
                        attempts, self.settings.retry_count, e, delay
                    );
                    warn!("{}", msg.trim());
                    let _ = log_file.write_all(msg.as_bytes()).await;
                    sleep(Duration::from_secs(delay)).await;
                }
            }
        }
    }

    async fn copy_with_retry(
        &self,
        paths: Vec<String>,
        log_file: &mut tokio::fs::File,
    ) -> Result<()> {
        let mut attempts = 0;
        loop {
            let res = self
                .nix_ops
                .copy_to_cache(
                    &paths,
                    &self.settings.cache_dir,
                    self.settings.secret_key_file.as_deref(),
                    log_file,
                )
                .await;
            match res {
                Ok(_) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if attempts >= self.settings.retry_count {
                        return Err(e);
                    }
                    let delay = self.settings.retry_delay_secs;
                    use tokio::io::AsyncWriteExt;
                    let msg = format!(
                        "Copy failed (attempt {}/{}): {}. Retrying in {}s...\n",
                        attempts, self.settings.retry_count, e, delay
                    );
                    warn!("{}", msg.trim());
                    let _ = log_file.write_all(msg.as_bytes()).await;
                    sleep(Duration::from_secs(delay)).await;
                }
            }
        }
    }

    async fn realise_with_retry(
        &self,
        paths: Vec<String>,
        log_file: &mut tokio::fs::File,
    ) -> Result<()> {
        let mut attempts = 0;
        loop {
            let res = self.nix_ops.realise(&paths, log_file).await;
            match res {
                Ok(_) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if attempts >= self.settings.retry_count {
                        return Err(e);
                    }
                    let delay = self.settings.retry_delay_secs;
                    use tokio::io::AsyncWriteExt;
                    let msg = format!(
                        "Realise failed (attempt {}/{}): {}. Retrying in {}s...\n",
                        attempts, self.settings.retry_count, e, delay
                    );
                    warn!("{}", msg.trim());
                    let _ = log_file.write_all(msg.as_bytes()).await;
                    sleep(Duration::from_secs(delay)).await;
                }
            }
        }
    }

    async fn process_host(
        &self,
        host: &str,
        flake_ref: &str,
        log_path: &Path,
        job_id: Uuid,
    ) -> Result<()> {
        let mut log_file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(log_path)
            .await?;

        use tokio::io::AsyncWriteExt;

        let timestamp = Local::now().format("%F %H:%M:%S.%3f");
        let msg = format!(
            "[{}] Building NixOS system for {} using {}\n",
            timestamp, host, flake_ref
        );
        info!("{}", msg.trim());
        log_file.write_all(msg.as_bytes()).await?;

        let arch = self.nix_ops.get_system_arch(flake_ref, host).await?;

        let cores = self.settings.arch_cores.get(&arch).copied();

        let result_path = match self
            .build_with_retry(flake_ref, host, cores, &mut log_file)
            .await
        {
            Ok(path) => path,
            Err(e) => {
                let err_msg = format!("Build failed: {}\n", e);
                error!("{}", err_msg.trim());
                log_file.write_all(err_msg.as_bytes()).await?;
                return Err(e);
            }
        };

        let msg = format!(
            "[{}] Built system: {}\n",
            Local::now().format("%F %H:%M:%S.%3f"),
            result_path
        );
        log_file.write_all(msg.as_bytes()).await?;

        let msg = format!(
            "[{}] Copying closure to cache...\n",
            Local::now().format("%F %H:%M:%S.%3f")
        );
        log_file.write_all(msg.as_bytes()).await?;

        self.copy_with_retry(vec![result_path.clone()], &mut log_file)
            .await?;

        let drv_path = self.nix_ops.get_drv_path(flake_ref, host).await?;

        let msg = format!(
            "[{}] calculating full closure...\n",
            Local::now().format("%F %H:%M:%S.%3f")
        );
        log_file.write_all(msg.as_bytes()).await?;

        let requisites = self
            .nix_ops
            .query_requisites(&drv_path)
            .await
            .context(format!(
                "Failed to query requisites for derivation: {}",
                drv_path
            ))?;

        let mut all_outputs = std::collections::HashSet::new();
        for path in &requisites {
            if path.ends_with(".drv") {
                let outputs = self
                    .nix_ops
                    .get_derivation_outputs(path)
                    .await
                    .context(format!(
                        "Failed to get outputs for derivation in first pass: {}",
                        path
                    ))
                    .unwrap_or_default();
                all_outputs.extend(outputs);
            }
        }

        let mut paths_to_copy = Vec::new();
        let mut paths_to_realise = Vec::new();
        let mut outputs_to_copy = Vec::new();

        for path in requisites {
            let hash = get_hash(&path).unwrap_or("");
            let narinfo = Path::new(&self.settings.cache_dir).join(format!("{}.narinfo", hash));

            if !all_outputs.contains(&path) && !narinfo.exists() {
                paths_to_copy.push(path.clone());
            }

            if path.ends_with(".drv") {
                let outputs = self
                    .nix_ops
                    .get_derivation_outputs(&path)
                    .await
                    .context(format!(
                        "Failed to get outputs for derivation in second pass: {}",
                        path
                    ))
                    .unwrap_or_default();
                for out in outputs {
                    let out_hash = get_hash(&out).unwrap_or("");
                    let out_narinfo =
                        Path::new(&self.settings.cache_dir).join(format!("{}.narinfo", out_hash));

                    if !out_narinfo.exists() {
                        paths_to_realise.push(path.clone());
                        outputs_to_copy.push(out);
                    }
                }
            }
        }

        if !paths_to_copy.is_empty() {
            let msg = format!(
                "[{}] Caching {} input paths...\n",
                Local::now().format("%F %H:%M:%S.%3f"),
                paths_to_copy.len()
            );
            log_file.write_all(msg.as_bytes()).await?;
            if let Err(e) = self.copy_with_retry(paths_to_copy, &mut log_file).await {
                let msg = format!("Failed to copy inputs: {}\n", e);
                log_file.write_all(msg.as_bytes()).await?;
            }
        }

        if !paths_to_realise.is_empty() {
            let msg = format!(
                "[{}] Realising and caching derivation outputs...\n",
                Local::now().format("%F %H:%M:%S.%3f")
            );
            log_file.write_all(msg.as_bytes()).await?;

            paths_to_realise.sort();
            paths_to_realise.dedup();

            if let Err(e) = self
                .realise_with_retry(paths_to_realise, &mut log_file)
                .await
            {
                let msg = format!("Failed to realise: {}\n", e);
                log_file.write_all(msg.as_bytes()).await?;
            }

            if !outputs_to_copy.is_empty() {
                outputs_to_copy.sort();
                outputs_to_copy.dedup();
                if let Err(e) = self.copy_with_retry(outputs_to_copy, &mut log_file).await {
                    let msg = format!("Failed to copy outputs: {}\n", e);
                    log_file.write_all(msg.as_bytes()).await?;
                }
            }
        }

        let msg = format!(
            "[{}] Finished {}\n",
            Local::now().format("%F %H:%M:%S.%3f"),
            host
        );
        log_file.write_all(msg.as_bytes()).await?;

        if let Some(mut job) = self.jobs.get_mut(&job_id) {
            if let Some(results) = &mut job.results {
                results.insert(host.to_string(), result_path.clone());
            } else {
                let mut map = std::collections::HashMap::new();
                map.insert(host.to_string(), result_path.clone());
                job.results = Some(map);
            }
        }

        Ok(())
    }
}

fn get_hash(path: &str) -> Option<&str> {
    Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .map(|s| &s[0..32])
}
