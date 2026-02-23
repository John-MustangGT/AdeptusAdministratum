use adeptus_administratum::{store::DatasheetStore, web::{build_router, state::AppState}};
use std::{net::SocketAddr, path::Path, sync::Arc};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialise structured logging (RUST_LOG=info by default).
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "adeptus_administratum=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load all datasheets from the data/ directory at startup.
    let data_dir = Path::new("data");
    if !data_dir.exists() {
        tracing::warn!(
            "data/ directory not found — no datasheets will be available. \
             Run from the project root or set the working directory appropriately."
        );
    }

    let store = DatasheetStore::load_from_dir(data_dir)?;
    tracing::info!(
        count = store.all().len(),
        "Loaded datasheets"
    );

    let state = AppState {
        store: Arc::new(store),
    };

    let app = build_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
