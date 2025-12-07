use anyhow::Result;
use metrics_exporter_prometheus::PrometheusBuilder;
use nix_local_cache::{api, config, logging, service};
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    logging::init();

    let settings = config::Settings::new()?;
    info!("Configuration loaded");

    let (service_instance, mut queue_rx) = service::BuildService::new(settings.clone()).await?;
    let service = Arc::new(service_instance);
    service.init().await?;

    let builder = PrometheusBuilder::new();
    let metrics_handle = builder
        .install_recorder()
        .expect("failed to install metrics recorder");

    let state = Arc::new(api::AppState::new(service.clone(), metrics_handle));

    // Start Worker
    let service_clone = service.clone();
    tokio::spawn(async move {
        info!("Worker started");
        while let Some(job_id) = queue_rx.recv().await {
            service_clone.process_job(job_id).await;
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
