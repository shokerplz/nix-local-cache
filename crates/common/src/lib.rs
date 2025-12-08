use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
#[cfg_attr(feature = "db", derive(sqlx::Type), sqlx(type_name = "TEXT"))]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[cfg_attr(feature = "db", derive(sqlx::FromRow))]
pub struct Job {
    pub id: Uuid,
    #[cfg_attr(feature = "db", sqlx(json))]
    pub hosts: Vec<String>,
    pub status: JobStatus,
    pub status_message: Option<String>,
    pub created_at: DateTime<Local>,
    pub started_at: Option<DateTime<Local>>,
    pub finished_at: Option<DateTime<Local>>,
    pub log_path: String,
    pub flake_ref: String,
    #[cfg_attr(feature = "db", sqlx(json))]
    pub results: Option<std::collections::HashMap<String, String>>,
    pub current_host: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct BuildRequest {
    pub hosts: Option<Vec<String>>,
    pub flake_url: Option<String>,
    pub flake_branch: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PaginatedJobs {
    pub jobs: Vec<Job>,
    pub total: i64,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
}
