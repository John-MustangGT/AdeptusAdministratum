use crate::web::session::load_roster;
use crate::web::state::AppState;
use axum::{
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use tower_sessions::Session;

pub async fn handler(State(_state): State<AppState>, session: Session) -> Response {
    let roster = load_roster(&session).await;
    let json = match serde_json::to_string_pretty(&roster) {
        Ok(j) => j,
        Err(e) => {
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                .into_response()
        }
    };
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"roster.json\"",
            ),
        ],
        json,
    )
        .into_response()
}
