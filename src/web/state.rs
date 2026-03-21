use crate::store::DatasheetStore;
use std::sync::Arc;

/// Shared application state injected into every Axum handler.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<DatasheetStore>,
}
