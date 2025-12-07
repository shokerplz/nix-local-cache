use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,nix_local_cache=debug".into());

    let fmt_layer = tracing_subscriber::fmt::layer()
        //.json() // Enable JSON logging for production
        .with_target(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}
