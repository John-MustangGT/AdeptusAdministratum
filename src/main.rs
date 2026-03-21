use administratio_militaris::{
    config::Config,
    store::DatasheetStore,
    web::{build_router, state::AppState},
};
use std::{net::SocketAddr, sync::Arc};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialise structured logging (RUST_LOG=info by default).
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "administratio_militaris=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration.
    let cfg = Config::load()?;
    tracing::info!(
        host = %cfg.server.host,
        port = cfg.server.port,
        tabularium_dir = %cfg.data.tabularium_dir.display(),
        "Configuration loaded"
    );

    // Load all datasheets from the configured directory at startup.
    let data_dir = &cfg.data.tabularium_dir;
    if !data_dir.exists() {
        tracing::warn!(
            dir = %data_dir.display(),
            "tabularium directory not found — no datasheets will be available. \
             Run from the project root or adjust tabularium_dir in config.toml."
        );
    }

    let store = DatasheetStore::load_from_dir(data_dir)?;
    tracing::info!(count = store.all().len(), "Loaded datasheets");

    let state = AppState {
        store: Arc::new(store),
    };

    let app = build_router(state);

    let addr = SocketAddr::new(cfg.server.host, cfg.server.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Print a clear startup banner to stdout.
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Administratio Militaris");
    println!("  Listening on  http://{addr}");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    tracing::info!(%addr, "Server started");
    axum::serve(listener, app).await?;

    Ok(())
}
