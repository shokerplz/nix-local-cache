use crate::service::BuildService;
use crate::nix;
use nix_local_cache_common::BuildRequest;
use axum::{
    extract::{Path, Query, State},
    http::{Method, StatusCode},
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
};
use futures::Stream;
use metrics_exporter_prometheus::PrometheusHandle;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

pub struct AppState {
    pub service: Arc<BuildService>,
    pub metrics_handle: PrometheusHandle,
}

pub fn app(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);

    Router::new()
        .route("/health", get(health))
        .route("/build", post(trigger_build))
        .route("/flake/hosts", get(get_flake_hosts))
        .route("/jobs", get(list_jobs))
        .route("/jobs/:id", get(get_job_status))
        .route("/jobs/:id/cancel", post(cancel_job))
        .route("/jobs/:id/restart", post(restart_job))
        .route("/jobs/:id/logs", get(stream_job_logs))
        .route("/jobs/:id/logs/range", get(get_job_logs_range))
        .route("/logs", get(list_logs))
        .route("/logs/:name", get(get_log))
        .route("/metrics", get(metrics))
        .layer(cors)
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

async fn metrics(State(state): State<Arc<AppState>>) -> String {
    state.metrics_handle.render()
}

#[derive(Deserialize)]
struct GetHostsQuery {
    flake_url: Option<String>,
    branch: Option<String>,
}

async fn get_flake_hosts(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GetHostsQuery>,
) -> impl IntoResponse {
    let flake_ref = nix::resolve_flake_ref(query.flake_url, query.branch, &state.service.settings.flake_path);

    match nix::get_hosts(&flake_ref).await {
        Ok(hosts) => Json(hosts).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn list_jobs(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut jobs: Vec<_> = state.service.jobs.iter().map(|entry| entry.value().clone()).collect();
    // Sort by created_at descending
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(jobs).into_response()
}

async fn trigger_build(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BuildRequest>,
) -> impl IntoResponse {
    match state.service.submit_build(payload).await {
        Ok(id) => {
            metrics::counter!("build_requests_total").increment(1);
            (StatusCode::ACCEPTED, Json(json!({ "job_id": id }))).into_response()
        }
        Err(e) => {
            tracing::error!("Failed to submit build: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

async fn get_job_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if let Some(job) = state.service.jobs.get(&id) {
        Json(job.clone()).into_response()
    } else {
        (StatusCode::NOT_FOUND, "Job not found").into_response()
    }
}

async fn cancel_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.service.cancel_job(id).await {
        Ok(_) => (StatusCode::OK, "Job cancelled").into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn restart_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.service.restart_job(id).await {
        Ok(_) => (StatusCode::OK, "Job restarted").into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct LogRangeQuery {
    #[serde(default)]
    from_line: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    tail: bool,
}

fn default_limit() -> usize {
    200
}

#[derive(Debug, Serialize)]
struct LogRangeResponse {
    lines: Vec<String>,
    total_lines: usize,
    from_line: usize,
    returned_lines: usize,
}

async fn get_job_logs_range(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(params): Query<LogRangeQuery>,
) -> impl IntoResponse {
    let job = match state.service.jobs.get(&id) {
        Some(j) => j.clone(),
        None => return (StatusCode::NOT_FOUND, "Job not found").into_response(),
    };

    let log_path = std::path::Path::new(&state.service.settings.log_dir).join(&job.log_path);

    // Read the log file and count total lines
    let file = match fs::File::open(&log_path) {
        Ok(f) => f,
        Err(_) => {
            return Json(LogRangeResponse {
                lines: vec![],
                total_lines: 0,
                from_line: 0,
                returned_lines: 0,
            })
            .into_response()
        }
    };

    let reader = BufReader::new(file);
    let all_lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
    let total_lines = all_lines.len();

    // Limit to max 500 lines per request
    let limit = params.limit.min(500);

    let (from_line, lines) = if params.tail {
        // Get last N lines
        let start = if total_lines > limit {
            total_lines - limit
        } else {
            0
        };
        (start, all_lines[start..].to_vec())
    } else {
        // Get lines from specified position
        let start = params.from_line.min(total_lines);
        let end = (start + limit).min(total_lines);
        (start, all_lines[start..end].to_vec())
    };

    let returned_lines = lines.len();

    Json(LogRangeResponse {
        lines,
        total_lines,
        from_line,
        returned_lines,
    })
    .into_response()
}

async fn stream_job_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let job = match state.service.jobs.get(&id) {
        Some(j) => j.clone(),
        None => return (StatusCode::NOT_FOUND, "Job not found").into_response(),
    };

    let log_path = std::path::Path::new(&state.service.settings.log_dir).join(&job.log_path);

    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, std::io::Error>> + Send>> =
        Box::pin(async_stream::try_stream! {
            let mut file = match tokio::fs::File::open(&log_path).await {
                Ok(f) => f,
                Err(_) => {
                    yield Event::default().data("Log file not found");
                    return;
                }
            };

            let mut interval = tokio::time::interval(Duration::from_secs(1));
            let mut pos = 0;

            loop {
                interval.tick().await;

                let metadata = match file.metadata().await {
                    Ok(m) => m,
                    Err(_) => break,
                };

                if metadata.len() > pos {
                     let mut buffer = vec![0; (metadata.len() - pos) as usize];
                     if (file.read_exact(&mut buffer).await).is_ok() {
                         pos += buffer.len() as u64;
                         let text = String::from_utf8_lossy(&buffer);
                         for line in text.lines() {
                             yield Event::default().data(line);
                         }
                     }
                }

                // Check if job finished
                if let Some(current_job) = state.service.jobs.get(&id) {
                    match current_job.status {
                         nix_local_cache_common::JobStatus::Completed | nix_local_cache_common::JobStatus::Failed => {
                             let metadata = file.metadata().await;
                             if let Ok(m) = metadata {
                                 if m.len() <= pos {
                                     break;
                                 }
                             } else {
                                 break;
                             }
                         }
                         _ => {}
                    }
                }
            }
        });

    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

async fn list_logs(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let log_dir = &state.service.settings.log_dir;
    match fs::read_dir(log_dir) {
        Ok(entries) => {
            let files: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            Json(files).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_log(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let log_path = std::path::Path::new(&state.service.settings.log_dir).join(&name);

    // Security check: prevent traversal
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return (StatusCode::BAD_REQUEST, "Invalid filename").into_response();
    }

    match fs::read_to_string(log_path) {
        Ok(content) => content.into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "Log not found").into_response(),
    }
}

impl AppState {
    pub fn new(service: Arc<BuildService>, metrics_handle: PrometheusHandle) -> Self {
        Self {
            service,
            metrics_handle,
        }
    }
}
