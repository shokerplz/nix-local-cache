mod api;
mod config;
mod logging;
mod nix;
mod service;

use anyhow::Result;
use metrics_exporter_prometheus::PrometheusBuilder;
use std::sync::Arc;
use tracing::info;
// use crate::{api, config, logging, service};

#[tokio::main]
async fn main() -> Result<()> {
    logging::init();

    let settings = config::Settings::new()?;
    info!("Configuration loaded");

    let nix_ops = Arc::new(nix::RealNixOps);
    let (service_instance, mut queue_rx) = service::BuildService::new(settings.clone(), nix_ops).await?;
    let service = Arc::new(service_instance);
    service.init().await?;

    let builder = PrometheusBuilder::new();
    let metrics_handle = builder
        .install_recorder()
        .expect("failed to install metrics recorder");

    let state = Arc::new(api::AppState::new(service.clone(), metrics_handle));

    // Start Worker
    let service_clone = service.clone();
    let worker_count = settings.worker_threads;
    
    tokio::spawn(async move {
        info!("Worker started with {} threads", worker_count);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(worker_count));
        
        while let Some(job_id) = queue_rx.recv().await {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let service = service_clone.clone();
            
            let handle = tokio::spawn(async move {
                service.process_job(job_id).await;
                // Cleanup handle from map when done
                service.running_jobs.remove(&job_id);
                drop(permit);
            });
            
            service_clone.running_jobs.insert(job_id, handle);
        }
    });

    // Start API
    let app = api::app(state.clone());
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], settings.port));

    info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app).await?;

    Ok(())
}
