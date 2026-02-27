use crate::datasheet::UnitDatasheet;
use crate::web::state::AppState;
use askama::Template;
use askama_axum::IntoResponse;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Response,
};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Unit browser
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct ListQuery {
    pub faction: Option<String>,
    pub game_system: Option<String>,
}

#[derive(Template)]
#[template(path = "datasheets/list.html")]
struct ListTemplate {
    units: Vec<UnitDatasheetSummary>,
    factions: Vec<String>,
    active_faction: Option<String>,
}

struct UnitDatasheetSummary {
    id: String,
    name: String,
    faction: String,
    battlefield_role: String,
    base_points: u32,
    unit_min: u32,
    unit_max: u32,
}

impl From<&UnitDatasheet> for UnitDatasheetSummary {
    fn from(ds: &UnitDatasheet) -> Self {
        Self {
            id: ds.id.clone(),
            name: ds.name.clone(),
            faction: ds.faction.primary.clone(),
            battlefield_role: ds.battlefield_role.clone(),
            base_points: ds.points.base,
            unit_min: ds.unit_size.min,
            unit_max: ds.unit_size.max,
        }
    }
}

pub async fn list_handler(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> impl IntoResponse {
    let units: Vec<UnitDatasheetSummary> = if let Some(ref faction) = query.faction {
        state
            .store
            .by_faction(faction)
            .iter()
            .map(|ds| UnitDatasheetSummary::from(*ds))
            .collect()
    } else {
        state
            .store
            .all()
            .iter()
            .map(|ds| UnitDatasheetSummary::from(*ds))
            .collect()
    };

    ListTemplate {
        units,
        factions: state.store.factions(),
        active_faction: query.faction,
    }
}

// ---------------------------------------------------------------------------
// Single unit detail
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "datasheets/detail.html")]
struct DetailTemplate {
    ds: UnitDatasheet,
}

pub async fn detail_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match state.store.get(&id) {
        Some(ds) => DetailTemplate { ds: ds.clone() }.into_response(),
        None => (StatusCode::NOT_FOUND, "Datasheet not found").into_response(),
    }
}
