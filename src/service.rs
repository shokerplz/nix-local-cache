use crate::config::Settings;
use crate::nix;
use crate::types::{BuildRequest, Job, JobStatus};
use anyhow::Result;
use chrono::Local;
use dashmap::DashMap;
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
}

impl BuildService {
    pub async fn new(settings: Settings) -> Result<(Self, mpsc::Receiver<Uuid>)> {
        let (tx, rx) = mpsc::channel(100);
        
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

        if !Path::new(&self.settings.meta_file).exists() {
            File::create(&self.settings.meta_file)?;
        }

        sqlx::migrate!().run(&self.db_pool).await?;

        self.load_jobs_from_db().await?;

        Ok(())
    }

    pub async fn cancel_job(&self, job_id: Uuid) -> Result<()> {
        if let Some((_, handle)) = self.running_jobs.remove(&job_id) {
            handle.abort();
            info!("Cancelled job {}", job_id);
            
            // Update status to failed/cancelled
             if let Some(mut job) = self.jobs.get_mut(&job_id) {
                job.status = JobStatus::Failed;
                job.status_message = Some("Cancelled by user".to_string());
                job.finished_at = Some(Local::now());
                
                // We need to update DB. But we are in async fn, so we can await.
                // However, we hold a lock on `job`.
                // Clone needed data and drop lock before await?
                // Or just use the update helper which takes &Job?
                // `update_job_in_db` takes &Job.
                // But we hold the lock on `job` (RefMut).
                // `update_job_in_db` is async, so we can't await while holding non-async-aware lock if we were using std::sync::Mutex.
                // But DashMap uses parking_lot (sync). Awaiting while holding it will deadlock if update_job_in_db tries to access dashmap?
                // No, update_job_in_db only touches DB.
                // BUT, awaiting while holding a sync lock is bad practice (blocks other threads).
                // Better: Modify, drop lock, then update DB.
                let job_clone = job.clone();
                drop(job);
                self.update_job_in_db(&job_clone).await?;
            }
            Ok(())
        } else {
            // Job might be queued but not running yet?
            // If queued, we should remove it from queue?
            // MPSC queue doesn't support removal.
            // But we can mark it as cancelled in DB/Map, and when worker picks it up, it checks status?
            if let Some(mut job) = self.jobs.get_mut(&job_id) {
                if matches!(job.status, JobStatus::Queued) {
                    job.status = JobStatus::Failed;
                    job.status_message = Some("Cancelled by user".to_string());
                    job.finished_at = Some(Local::now());
                    let job_clone = job.clone();
                    drop(job);
                    self.update_job_in_db(&job_clone).await?;
                    info!("Cancelled queued job {}", job_id);
                    return Ok(());
                }
            }
            Err(anyhow::anyhow!("Job not running or not found"))
        }
    }

    async fn load_jobs_from_db(&self) -> Result<()> {
        let rows = sqlx::query("SELECT * FROM jobs")
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
                results: row.try_get("results").unwrap_or_else(|_| None).and_then(|v: String| serde_json::from_str(&v).ok()),
            };
            jobs.push(job);
        }

        for mut job in jobs {
            // If job was Running when we crashed, mark it Failed
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
        sqlx::query!(
            r#"
            INSERT INTO jobs (id, hosts, status, status_message, created_at, started_at, finished_at, log_path, flake_ref, results)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            job.id,
            hosts_json,
            status,
            job.status_message,
            job.created_at,
            job.started_at,
            job.finished_at,
            job.log_path,
            job.flake_ref,
            results_json
        )
        .execute(&self.db_pool)
        .await?;
        Ok(())
    }

    async fn update_job_in_db(&self, job: &Job) -> Result<()> {
        let hosts_json = sqlx::types::Json(&job.hosts);
        let status = job.status.clone();
        let results_json = sqlx::types::Json(&job.results);
        sqlx::query!(
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
                results = ?
            WHERE id = ?
            "#,
            hosts_json,
            status,
            job.status_message,
            job.created_at,
            job.started_at,
            job.finished_at,
            job.log_path,
            job.flake_ref,
            results_json,
            job.id
        )
        .execute(&self.db_pool)
        .await?;
        Ok(())
    }

    pub async fn submit_build(&self, req: BuildRequest) -> Result<Uuid> {
        let id = Uuid::new_v4();

        // Determine flake reference
        let flake_ref = if let Some(mut url) = req.flake_url {
            // Basic heuristic to convert SCP-style SSH URLs to Nix syntax
            // e.g. git@github.com:user/repo.git -> git+ssh://git@github.com/user/repo.git
            if !url.contains("://") && url.contains('@') {
                url = format!("git+ssh://{}", url.replace(":", "/"));
            } else if url.starts_with("https://") && url.ends_with(".git") {
                url = format!("git+{}", url);
            }

            if let Some(branch) = req.flake_branch {
                format!("{}?ref={}", url, branch)
            } else {
                url
            }
        } else {
            self.settings.flake_path.clone()
        };

        let target_hosts = if let Some(h) = req.hosts {
            h
        } else {
            nix::get_hosts(&flake_ref).await?
        };

        let log_file_name = format!("{}.log", id);
        let log_path = Path::new(&self.settings.log_dir).join(&log_file_name);
        // Create empty log file
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
            results: Some(std::collections::HashMap::new()),
        };

        self.jobs.insert(id, job.clone());
        self.insert_job_into_db(&job).await?;
        self.queue_tx.send(id).await?;

        Ok(id)
    }

    pub async fn process_job(&self, job_id: Uuid) {
        // Check if job was cancelled while in queue
        if let Some(job) = self.jobs.get(&job_id) {
            if matches!(job.status, JobStatus::Failed) && job.status_message.as_deref() == Some("Cancelled by user") {
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
        
        // Update in DB before releasing lock for long operations
        self.update_job_in_db(&job).await.unwrap_or_else(|e| {
            error!("Failed to update job {} in DB: {}", job_id, e);
        });

        // Release lock before long operation
        let hosts = job.hosts.clone();
        let log_path_str = job.log_path.clone();
        let flake_ref = job.flake_ref.clone();
        drop(job);

        let log_full_path = Path::new(&self.settings.log_dir).join(&log_path_str);

        info!(
            "Starting job {} for hosts {:?} using flake {}",
            job_id, hosts, flake_ref
        );

        let mut success = true;
        let mut status_message: Option<String> = None;
        for host in hosts {
            // Check for cancellation before processing each host
            // (Though if cancelled, the task should be aborted, so this might be redundant but safe)
             if !self.running_jobs.contains_key(&job_id) {
                 // Actually this check is tricky because we ARE running.
                 // If we were aborted, we wouldn't be here.
                 // So this check is likely useless inside the task itself.
             }

            if let Err(e) = self.process_host(&host, &flake_ref, &log_full_path, job_id).await {
                let msg = format!("Job {} failed for host {}: {}", job_id, host, e);
                error!("{}", msg);
                status_message = Some(msg);
                success = false;
            }
        }

        let mut job = self.jobs.get_mut(&job_id).unwrap();
        job.finished_at = Some(Local::now());
        job.status = if success {
            JobStatus::Completed
        } else {
            JobStatus::Failed
        };
        job.status_message = status_message;

        info!("Job {} finished with status {:?}", job_id, job.status);
        
        // Update in DB after job is finished
        self.update_job_in_db(&job).await.unwrap_or_else(|e| {
            error!("Failed to update job {} in DB: {}", job_id, e);
        });
    }

    // Helper to avoid closure borrowing issues
    async fn build_with_retry(
        &self,
        flake_ref: &str,
        host: &str,
        cores: Option<u32>,
        log_file: &mut tokio::fs::File,
    ) -> Result<String> {
        let mut attempts = 0;
        loop {
            let res = nix::build_system(flake_ref, host, cores, self.settings.builders.as_deref(), log_file).await;
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

    async fn copy_with_retry(&self, paths: Vec<String>, log_file: &mut tokio::fs::File) -> Result<()> {
        let mut attempts = 0;
        loop {
            // Clone paths for the attempt
            let res = nix::copy_to_cache(&paths, &self.settings.cache_dir, self.settings.secret_key_file.as_deref(), log_file).await;
            match res {
                Ok(_) => return Ok(()),
                Err(e) => {
                    attempts += 1;
                    if attempts >= self.settings.retry_count {
                        return Err(e);
                    }
                    let delay = self.settings.retry_delay_secs;
                    use tokio::io::AsyncWriteExt;
                    let msg = format!("Copy failed (attempt {}/{}): {}. Retrying in {}s...\n", attempts, self.settings.retry_count, e, delay);
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
            let res = nix::realise(&paths, log_file).await;
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

    async fn process_host(&self, host: &str, flake_ref: &str, log_path: &Path, job_id: Uuid) -> Result<()> {
        // Use tokio::fs::File for async writing
        let mut log_file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(log_path)
            .await?;

        use tokio::io::AsyncWriteExt;

        let timestamp = Local::now().format("%F_%H-%M-%S.%3f");
        let msg = format!(
            "[{}] Building NixOS system for {} using {}\n",
            timestamp, host, flake_ref
        );
        info!("{}", msg.trim());
        log_file.write_all(msg.as_bytes()).await?;

        let arch = nix::get_system_arch(flake_ref, host).await?;

        // Use configured cores or default
        let cores = self.settings.arch_cores.get(&arch).copied();

        // Manual retry loop to satisfy borrow checker
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
            Local::now().format("%F_%H-%M-%S.%3f"),
            result_path
        );
        log_file.write_all(msg.as_bytes()).await?;

        let msg = format!(
            "[{}] Copying closure to cache...\n",
            Local::now().format("%F_%H-%M-%S.%3f")
        );
        log_file.write_all(msg.as_bytes()).await?;

        self.copy_with_retry(vec![result_path.clone()], &mut log_file)
            .await?;

        let drv_path = nix::get_drv_path(flake_ref, host).await?;

        let msg = format!(
            "[{}] calculating full closure...\n",
            Local::now().format("%F_%H-%M-%S.%3f")
        );
        log_file.write_all(msg.as_bytes()).await?;

        let requisites = nix::query_requisites(&drv_path).await?;

        // Batch paths for copying
        let mut paths_to_copy = Vec::new();
        let mut paths_to_realise = Vec::new();
        let mut outputs_to_copy = Vec::new();

        for path in requisites {
            let hash = get_hash(&path).unwrap_or("");
            let narinfo = Path::new(&self.settings.cache_dir).join(format!("{}.narinfo", hash));

            if !narinfo.exists() {
                paths_to_copy.push(path.clone());
            }

            if path.ends_with(".drv") {
                let outputs = nix::get_derivation_outputs(&path).await.unwrap_or_default();
                for out in outputs {
                    let out_hash = get_hash(&out).unwrap_or("");
                    let out_narinfo =
                        Path::new(&self.settings.cache_dir).join(format!("{}.narinfo", out_hash));

                    if !out_narinfo.exists() {
                        // We can't batch realise efficiently if we need to check output existence for each
                        // But we can collect ALL drvs that need ANY output realised.
                        // Simpler: just add to list.
                        // Note: `paths_to_realise` contains drv paths. `outputs_to_copy` contains output paths.
                        paths_to_realise.push(path.clone());
                        outputs_to_copy.push(out);
                    }
                }
            }
        }

        if !paths_to_copy.is_empty() {
            let msg = format!(
                "[{}] Caching {} input paths...\n",
                Local::now().format("%F_%H-%M-%S.%3f"),
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
                Local::now().format("%F_%H-%M-%S.%3f")
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
            Local::now().format("%F_%H-%M-%S.%3f"),
            host
        );
        log_file.write_all(msg.as_bytes()).await?;

        self.update_metadata(host, &result_path).await?;

        // Update results in job
        // Note: we need to lock the job again to update it.
        if let Some(mut job) = self.jobs.get_mut(&job_id) {
             if let Some(results) = &mut job.results {
                 results.insert(host.to_string(), result_path.clone());
             } else {
                 let mut map = std::collections::HashMap::new();
                 map.insert(host.to_string(), result_path.clone());
                 job.results = Some(map);
             }
             // We'll persist this to DB when the job status is updated at the end of process_job
             // OR we can update it incrementally here.
             // Let's update incrementally so UI sees it immediately if we wanted to.
             // But `process_job` logic at the end writes to DB.
             // However, `process_job` only writes status at the very end.
             // If we have multiple hosts, we might want to see progress.
             // For now, let's just update memory, and let the final DB update persist it.
             // Actually, `process_job` does `self.update_job_in_db(&job)` at the end.
             // But `job` in `process_job` (the variable) is a *clone* or *ref*?
             // `process_job` gets `let mut job = self.jobs.get_mut(...)`.
             // Then it drops it.
             // Then it iterates hosts.
             // So we need to re-acquire lock here to update `results`.
        }

        Ok(())
    }

    async fn update_metadata(&self, host: &str, store_path: &str) -> Result<()> {
        let entry = serde_json::json!({
            "host": host,
            "timestamp": Local::now().format("%F_%H-%M-%S.%3f").to_string(),
            "storePath": store_path
        });

        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(&self.settings.meta_file)?;

        writeln!(f, "{}", entry)?;

        Ok(())
    }
}

fn get_hash(path: &str) -> Option<&str> {
    Path::new(path)
        .file_name()
        .and_then(|f| f.to_str())
        .map(|s| &s[0..32])
}
