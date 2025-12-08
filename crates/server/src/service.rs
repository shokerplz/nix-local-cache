use crate::config::Settings;
use crate::nix;
use nix_local_cache_common::{BuildRequest, Job, JobStatus};
use anyhow::{Context, Result};
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
    nix_ops: Arc<dyn crate::nix::NixOps>,
}

impl BuildService {
    pub async fn new(settings: Settings, nix_ops: Arc<dyn crate::nix::NixOps>) -> Result<(Self, mpsc::Receiver<Uuid>)> {
        let (tx, rx) = mpsc::channel(100);
        
        // Ensure DB file exists
        if !Path::new(&settings.sqlite_db_path).exists() {
            info!("Database file not found at {}, creating...", settings.sqlite_db_path);
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
            info!("Cancelled job {}", job_id);
            
            Ok(())
        } else {
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
        sqlx::query!(
            r#"
            INSERT INTO jobs (id, hosts, status, status_message, created_at, started_at, finished_at, log_path, flake_ref, results, current_host)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            results_json,
            job.current_host
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
                results = ?,
                current_host = ?
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
            job.current_host,
            job.id
        )
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
                results: row.try_get("results").unwrap_or_else(|_| None).and_then(|v: String| serde_json::from_str(&v).ok()),
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

        let flake_ref = nix::resolve_flake_ref(req.flake_url, req.flake_branch, &self.settings.flake_path);

        let target_hosts = if let Some(h) = req.hosts {
            h
        } else if let Some(h) = &self.settings.hosts {
            h.clone()
        } else {
            self.nix_ops.get_hosts(&flake_ref).await?
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
            current_host: None,
        };

        self.jobs.insert(id, job.clone());
        self.insert_job_into_db(&job).await?;
        self.queue_tx.send(id).await?;

        Ok(id)
    }

    pub async fn process_job(&self, job_id: Uuid) {
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

        let job_data = job.clone();
        drop(job);

        if let Err(e) = self.update_job_in_db(&job_data).await {
            error!("Failed to update job {} in DB: {}", job_id, e);
        }

        let hosts = job_data.hosts.clone();
        let log_path_str = job_data.log_path.clone();
        let flake_ref = job_data.flake_ref.clone();

        let log_full_path = Path::new(&self.settings.log_dir).join(&log_path_str);

        info!(
            "Starting job {} for hosts {:?} using flake {}",
            job_id, hosts, flake_ref
        );

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

            // Check for cancellation before processing each host
            // (Though if cancelled, the task should be aborted, so this might be redundant but safe)
             if !self.running_jobs.contains_key(&job_id) {
             }

            if let Err(e) = self.process_host(&host, &flake_ref, &log_full_path, job_id).await {
                let msg = format!("Job {} failed for host {}: {}", job_id, host, e);
                error!("{}", msg);
                status_message = Some(msg);
                success = false;
            }
        }

        let mut job = match self.jobs.get_mut(&job_id) {
            Some(j) => j,
            None => {
                error!("Job {} disappeared from memory while processing!", job_id);
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

        // Update in DB after job is finished
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
            let res = self.nix_ops.build_system(flake_ref, host, cores, self.settings.builders.as_deref(), log_file).await;
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
            let res = self.nix_ops.copy_to_cache(&paths, &self.settings.cache_dir, self.settings.secret_key_file.as_deref(), log_file).await;
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

    async fn process_host(&self, host: &str, flake_ref: &str, log_path: &Path, job_id: Uuid) -> Result<()> {
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

        let requisites = self.nix_ops.query_requisites(&drv_path)
            .await
            .context(format!("Failed to query requisites for derivation: {}", drv_path))?;

        let mut all_outputs = std::collections::HashSet::new();
        for path in &requisites {
            if path.ends_with(".drv") {
                let outputs = self.nix_ops.get_derivation_outputs(path)
                    .await
                    .context(format!("Failed to get outputs for derivation in first pass: {}", path))
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
                let outputs = self.nix_ops.get_derivation_outputs(&path)
                    .await
                    .context(format!("Failed to get outputs for derivation in second pass: {}", path))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use tempfile::tempdir;
    use crate::nix::MockNixOps;

    async fn create_test_service(nix_ops: Arc<MockNixOps>) -> (BuildService, mpsc::Receiver<Uuid>, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache_dir = temp_dir.path().join("cache");
        let log_dir = temp_dir.path().join("logs");

        let settings = Settings {
            port: 3000,
            sqlite_db_path: db_path.to_string_lossy().to_string(),
            cache_dir: cache_dir.to_string_lossy().to_string(),
            log_dir: log_dir.to_string_lossy().to_string(),
            flake_path: ".".to_string(),
            hosts: None,
            secret_key_file: None,
            retry_count: 1,
            retry_delay_secs: 1,
            arch_cores: std::collections::HashMap::new(),
            worker_threads: 1,
            builders: None,
        };

        let (service, rx) = BuildService::new(settings, nix_ops).await.unwrap();
        service.init().await.unwrap();
        (service, rx, temp_dir)
    }

    #[tokio::test]
    async fn test_restart_failed_job() {
        let mock_nix = Arc::new(MockNixOps::new());
        let (service, _rx, _temp_dir) = create_test_service(mock_nix).await;

        // Create a fake job
        let job_id = Uuid::new_v4();
        let log_file_name = format!("{}.log", job_id);
        // Create log file so restart_job can truncate it
        let log_path = Path::new(&service.settings.log_dir).join(&log_file_name);
        File::create(&log_path).unwrap();

        let job = Job {
            id: job_id,
            hosts: vec!["localhost".to_string()],
            status: JobStatus::Failed,
            status_message: Some("Failed".to_string()),
            created_at: Local::now(),
            started_at: Some(Local::now()),
            finished_at: Some(Local::now()),
            log_path: log_file_name,
            flake_ref: ".".to_string(),
            results: Some(std::collections::HashMap::new()),
            current_host: Some("localhost".to_string()),
        };

        service.jobs.insert(job_id, job.clone());
        service.insert_job_into_db(&job).await.unwrap();

        // Restart it
        service.restart_job(job_id).await.unwrap();

        // Verify status
        let updated_job = service.jobs.get(&job_id).unwrap();
        assert_eq!(updated_job.status, JobStatus::Queued);
        assert_eq!(updated_job.status_message, None);
        assert!(updated_job.started_at.is_none());
        assert!(updated_job.finished_at.is_none());
        assert!(updated_job.current_host.is_none());
        
        // Check DB update
        let saved_job = sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = ?")
            .bind(job_id)
            .fetch_one(&service.db_pool)
            .await
            .unwrap();
            
        assert_eq!(saved_job.status, JobStatus::Queued);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_job_lifecycle_success() {
        let mut mock_nix = MockNixOps::new();

        // Setup expectations
        mock_nix.expect_get_system_arch()
            .returning(|_, _| Box::pin(async { Ok("x86_64-linux".to_string()) }));
        
        mock_nix.expect_build_system()
            .returning(|_, _, _, _, _| Box::pin(async { Ok("/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-result".to_string()) }));
            
        mock_nix.expect_copy_to_cache()
            .returning(|_, _, _, _| Box::pin(async { Ok(()) }));
            
        mock_nix.expect_get_drv_path()
            .returning(|_, _| Box::pin(async { Ok("/nix/store/dddddddddddddddddddddddddddddddd-foo.drv".to_string()) }));
            
        mock_nix.expect_query_requisites()
            .returning(|_| Box::pin(async { Ok(vec!["/nix/store/dddddddddddddddddddddddddddddddd-foo.drv".to_string()]) }));
            
        mock_nix.expect_get_derivation_outputs()
            .returning(|_| Box::pin(async { Ok(vec!["/nix/store/oooooooooooooooooooooooooooooooo-out".to_string()]) }));
            
        mock_nix.expect_realise()
            .returning(|_, _| Box::pin(async { Ok(()) }));

        let mock_nix = Arc::new(mock_nix);
        let (service, mut rx, _temp_dir) = create_test_service(mock_nix.clone()).await;
        
        // Submit job
        let req = BuildRequest {
            hosts: Some(vec!["host1".to_string()]),
            flake_url: Some("github:owner/repo".to_string()),
            flake_branch: None,
        };
        let job_id = service.submit_build(req).await.unwrap();

        // Verify queued
        // Verify queued
        {
            let job = service.jobs.get(&job_id).unwrap();
            assert_eq!(job.status, JobStatus::Queued);
        } // Lock dropped here

        // Process job (simulate worker)
        // First, receive from queue
        let received_id = rx.recv().await.unwrap();
        assert_eq!(received_id, job_id);
        
        service.process_job(job_id).await;

        // Verify completed
        let job = service.jobs.get(&job_id).unwrap();
        assert_eq!(job.status, JobStatus::Completed);
        assert!(job.results.is_some());
        assert_eq!(job.results.as_ref().unwrap().get("host1").unwrap(), "/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-result");
    }
}
