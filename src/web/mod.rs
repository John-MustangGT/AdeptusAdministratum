pub mod routes;
pub mod session;
pub mod state;

use crate::web::state::AppState;
use axum::{
    routing::{get, post},
    Router,
};
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, SessionManagerLayer};

pub fn build_router(state: AppState) -> Router {
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store);

    Router::new()
        // Pages
        .route("/", get(routes::index::handler))
        .route("/datasheets", get(routes::datasheets::list_handler))
        .route("/datasheets/:id", get(routes::datasheets::detail_handler))
        // Roster
        .route("/roster", get(routes::roster::view_handler))
        .route("/roster/new", get(routes::roster::new_roster_form_handler))
        .route("/roster/new", post(routes::roster::create_roster_handler))
        .route("/roster/unit/add", post(routes::roster::add_unit_handler))
        .route(
            "/roster/unit/:entry_id/remove",
            post(routes::roster::remove_unit_handler),
        )
        .route(
            "/roster/unit/:entry_id/configure",
            post(routes::roster::configure_unit_handler),
        )
        .route("/roster/rename", post(routes::roster::rename_handler))
        // Export
        .route("/roster/export.json", get(routes::export::handler))
        // Static files
        .nest_service("/static", ServeDir::new("static"))
        .layer(session_layer)
        .with_state(state)
}
