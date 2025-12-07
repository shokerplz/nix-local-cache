use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, sqlx::Type)]
#[sqlx(type_name = "TEXT")] // Store JobStatus as TEXT in SQLite
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Job {
    pub id: Uuid,
    #[sqlx(json)] // Store Vec<String> as JSON text
    pub hosts: Vec<String>,
    pub status: JobStatus,
    pub status_message: Option<String>, // To store the reason for failure
    pub created_at: DateTime<Local>,
    pub started_at: Option<DateTime<Local>>,
    pub finished_at: Option<DateTime<Local>>,
    pub log_path: String,
    pub flake_ref: String,
}

#[derive(Debug, Deserialize)]
pub struct BuildRequest {
    pub hosts: Option<Vec<String>>,
    pub flake_url: Option<String>,
    pub flake_branch: Option<String>,
}
