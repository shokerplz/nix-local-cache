use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    let fmt_layer = tracing_subscriber::fmt::layer().with_target(true);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}
