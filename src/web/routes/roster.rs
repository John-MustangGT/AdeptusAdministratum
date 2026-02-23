use crate::datasheet::{Severity, UnitDatasheet};
use crate::roster::{RosterEntry, RosterList};
use crate::store::DatasheetStore;
use crate::validation::{calculate_points, validate_unit, UnitSelection};
use crate::web::session::{load_roster, save_roster};
use crate::web::state::AppState;
use askama::Template;
use askama_axum::IntoResponse;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Response,
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

// ---------------------------------------------------------------------------
// Owned view types for templates (avoids lifetime issues with borrowed refs)
// ---------------------------------------------------------------------------

struct OwnedOptionView {
    id: String,
    description: String,
    points_cost: i32,
    checked: bool,
    disabled: bool,
}

struct RosterEntryViewOwned {
    entry_id: String,
    datasheet_id: String,
    datasheet_name: String,
    battlefield_role: String,
    model_count: u32,
    unit_min: u32,
    unit_max: u32,
    unit_step: u32,
    options: Vec<OwnedOptionView>,
    issues: Vec<(String, String)>,
    points: u32,
}

fn enriched_selection(ds: &UnitDatasheet, selection: UnitSelection) -> UnitSelection {
    let mut sel = selection;
    if sel.active_keywords.is_empty() {
        sel.active_keywords = ds
            .keywords
            .unit
            .iter()
            .chain(ds.keywords.faction.iter())
            .cloned()
            .collect();
    }
    sel
}

fn build_entry_view(
    entry: &RosterEntry,
    ds: &UnitDatasheet,
    _store: &DatasheetStore,
) -> RosterEntryViewOwned {
    let sel = enriched_selection(ds, entry.selection.clone());
    let issues_raw = validate_unit(ds, &sel).unwrap_or_default();
    let points = calculate_points(ds, &sel);

    let issues = issues_raw
        .iter()
        .map(|i| {
            let cls = match i.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            (cls.to_string(), i.message.clone())
        })
        .collect();

    let options = ds
        .wargear_options
        .iter()
        .map(|opt| {
            let checked = sel.chosen_options.contains(&opt.id);
            let disabled = opt
                .mutually_exclusive_with
                .iter()
                .any(|excl_id| sel.chosen_options.contains(excl_id));
            OwnedOptionView {
                id: opt.id.clone(),
                description: opt.description.clone(),
                points_cost: opt.points_cost,
                checked,
                disabled,
            }
        })
        .collect();

    RosterEntryViewOwned {
        entry_id: entry.entry_id.clone(),
        datasheet_id: entry.datasheet_id.clone(),
        datasheet_name: ds.name.clone(),
        battlefield_role: ds.battlefield_role.clone(),
        model_count: sel.model_count,
        unit_min: ds.unit_size.min,
        unit_max: ds.unit_size.max,
        unit_step: ds.unit_size.step,
        options,
        issues,
        points,
    }
}

// ---------------------------------------------------------------------------
// Full roster view
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "roster/view.html")]
struct RosterViewTemplate {
    roster_name: String,
    entries: Vec<RosterEntryViewOwned>,
    total_points: u32,
    has_errors: bool,
    all_datasheets: Vec<(String, String)>,
}

pub async fn view_handler(
    State(state): State<AppState>,
    session: Session,
) -> impl IntoResponse {
    let roster = load_roster(&session).await;
    render_roster_view(&roster, &state.store)
}

fn render_roster_view(roster: &RosterList, store: &DatasheetStore) -> RosterViewTemplate {
    let entries: Vec<RosterEntryViewOwned> = roster
        .entries
        .iter()
        .filter_map(|entry| {
            store
                .get(&entry.datasheet_id)
                .map(|ds| build_entry_view(entry, ds, store))
        })
        .collect();

    let total_points: u32 = entries.iter().map(|e| e.points).sum();
    let has_errors = entries
        .iter()
        .any(|e| e.issues.iter().any(|(cls, _)| cls == "error"));

    let all_datasheets = store
        .all()
        .iter()
        .map(|ds| (ds.id.clone(), ds.name.clone()))
        .collect();

    RosterViewTemplate {
        roster_name: roster.name.clone(),
        entries,
        total_points,
        has_errors,
        all_datasheets,
    }
}

// ---------------------------------------------------------------------------
// Add unit
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AddUnitForm {
    datasheet_id: String,
}

pub async fn add_unit_handler(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<AddUnitForm>,
) -> Response {
    let ds = match state.store.get(&form.datasheet_id) {
        Some(d) => d,
        None => return (StatusCode::BAD_REQUEST, "Unknown datasheet").into_response(),
    };

    let mut roster = load_roster(&session).await;
    roster.add_unit(&form.datasheet_id, ds.unit_size.min);
    save_roster(&session, &roster).await;

    render_roster_view(&roster, &state.store).into_response()
}

// ---------------------------------------------------------------------------
// Remove unit
// ---------------------------------------------------------------------------

pub async fn remove_unit_handler(
    State(state): State<AppState>,
    session: Session,
    Path(entry_id): Path<String>,
) -> impl IntoResponse {
    let mut roster = load_roster(&session).await;
    roster.remove_unit(&entry_id);
    save_roster(&session, &roster).await;
    render_roster_view(&roster, &state.store)
}

// ---------------------------------------------------------------------------
// Configure unit (model count + wargear) — returns updated unit card partial
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct ConfigureUnitForm {
    #[serde(default)]
    model_count: Option<u32>,
    /// Repeated field: one value per checked wargear option.
    #[serde(default, rename = "options")]
    chosen_options: Vec<String>,
}

#[derive(Template)]
#[template(path = "roster/_unit_card.html")]
struct UnitCardTemplate {
    entry: RosterEntryViewOwned,
}

pub async fn configure_unit_handler(
    State(state): State<AppState>,
    session: Session,
    Path(entry_id): Path<String>,
    Form(form): Form<ConfigureUnitForm>,
) -> Response {
    let mut roster = load_roster(&session).await;

    let entry = match roster.get_entry_mut(&entry_id) {
        Some(e) => e,
        None => return (StatusCode::NOT_FOUND, "Entry not found").into_response(),
    };

    if let Some(count) = form.model_count {
        entry.selection.model_count = count;
    }
    entry.selection.chosen_options = form.chosen_options;

    let datasheet_id = entry.datasheet_id.clone();
    save_roster(&session, &roster).await;

    let ds = match state.store.get(&datasheet_id) {
        Some(d) => d,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "Datasheet gone").into_response(),
    };

    let entry = roster.entries.iter().find(|e| e.entry_id == entry_id).unwrap();
    let view = build_entry_view(entry, ds, &state.store);

    UnitCardTemplate { entry: view }.into_response()
}

// ---------------------------------------------------------------------------
// Rename roster
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RenameForm {
    roster_name: String,
}

pub async fn rename_handler(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<RenameForm>,
) -> impl IntoResponse {
    let mut roster = load_roster(&session).await;
    roster.name = form.roster_name;
    save_roster(&session, &roster).await;
    render_roster_view(&roster, &state.store)
}
