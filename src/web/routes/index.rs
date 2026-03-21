use crate::web::state::AppState;
use askama::Template;
use askama_axum::IntoResponse;
use axum::extract::State;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    factions: Vec<String>,
    game_systems: Vec<String>,
}

pub async fn handler(State(state): State<AppState>) -> impl IntoResponse {
    IndexTemplate {
        factions: state.store.factions(),
        game_systems: state.store.game_systems(),
    }
}
