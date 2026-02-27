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
    response::{Redirect, Response},
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

// ---------------------------------------------------------------------------
// Role ordering for the unit browser
// ---------------------------------------------------------------------------

const ROLE_ORDER: &[&str] = &[
    "HQ",
    "Troops",
    "Battleline",
    "Elites",
    "Fast Attack",
    "Heavy Support",
    "Dedicated Transport",
    "Flyer",
    "Lord of War",
    "Fortification",
];

fn role_sort_key(role: &str) -> usize {
    ROLE_ORDER
        .iter()
        .position(|&r| r == role)
        .unwrap_or(ROLE_ORDER.len())
}

// ---------------------------------------------------------------------------
// View types
// ---------------------------------------------------------------------------

struct UnitPickerItem {
    id: String,
    name: String,
    base_points: u32,
    is_allied: bool,
}

struct UnitGroup {
    role: String,
    units: Vec<UnitPickerItem>,
}

struct OwnedOptionView {
    id: String,
    description: String,
    points_cost: i32,
    checked: bool,
    disabled: bool,
    /// Comma-separated IDs of mutually exclusive options, used by the
    /// client-side live-exclusion JS to grey out conflicting checkboxes
    /// immediately (before the configure debounce fires).
    mutually_exclusive_ids: String,
}

struct RosterEntryViewOwned {
    entry_id: String,
    datasheet_id: String,
    datasheet_name: String,
    /// Player's custom label for this unit instance (empty string = not set).
    custom_name_str: String,
    battlefield_role: String,
    model_count: u32,
    unit_min: u32,
    unit_max: u32,
    unit_step: u32,
    options: Vec<OwnedOptionView>,
    issues: Vec<(String, String)>,
    points: u32,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn build_entry_view(entry: &RosterEntry, ds: &UnitDatasheet) -> RosterEntryViewOwned {
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
                mutually_exclusive_ids: opt.mutually_exclusive_with.join(","),
            }
        })
        .collect();

    RosterEntryViewOwned {
        entry_id: entry.entry_id.clone(),
        datasheet_id: entry.datasheet_id.clone(),
        datasheet_name: ds.name.clone(),
        custom_name_str: entry.custom_name.clone().unwrap_or_default(),
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

/// Deserialised form of a single unit's current DOM configure state, injected
/// by the `htmx:configRequest` JS hook in base.html for every roster mutation.
#[derive(Deserialize, Default)]
struct ConfigureStateEntry {
    entry_id: String,
    #[serde(default)]
    model_count: u32,
    #[serde(default)]
    chosen_options: Vec<String>,
    #[serde(default)]
    custom_name: String,
}

/// Apply DOM-sourced configure state to the roster before a mutation.
///
/// This ensures that when the user clicks Add/Remove/Move, the server sees the
/// latest wargear selections from the page rather than whatever was last flushed
/// to the session — closing the race window where a pending configure hadn't
/// yet fired (or was cancelled by the DOM swap).
fn apply_current_configure(roster: &mut RosterList, json: &str) {
    if json.is_empty() {
        return;
    }
    let states: Vec<ConfigureStateEntry> = match serde_json::from_str(json) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("apply_current_configure: failed to parse JSON: {:?}", e);
            return;
        }
    };
    for state in states {
        if let Some(entry) = roster.get_entry_mut(&state.entry_id) {
            if state.model_count > 0 {
                entry.selection.model_count = state.model_count;
            }
            entry.selection.chosen_options = state.chosen_options;
            entry.custom_name = if state.custom_name.is_empty() {
                None
            } else {
                Some(state.custom_name)
            };
        }
    }
}

fn build_unit_groups(store: &DatasheetStore, roster: &RosterList) -> Vec<UnitGroup> {
    let primary = roster.faction.as_str();
    let picker_units = store.units_for_roster(&roster.game_system, primary);

    let mut groups_map: std::collections::HashMap<String, Vec<UnitPickerItem>> =
        std::collections::HashMap::new();

    for ds in picker_units {
        groups_map
            .entry(ds.battlefield_role.clone())
            .or_default()
            .push(UnitPickerItem {
                id: ds.id.clone(),
                name: ds.name.clone(),
                base_points: ds.points.base,
                is_allied: ds.faction.primary != primary,
            });
    }

    let mut groups: Vec<UnitGroup> = groups_map
        .into_iter()
        .map(|(role, mut units)| {
            units.sort_by(|a, b| a.name.cmp(&b.name));
            UnitGroup { role, units }
        })
        .collect();

    groups.sort_by_key(|g| role_sort_key(&g.role));
    groups
}

// ---------------------------------------------------------------------------
// New roster form
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "roster/new.html")]
struct NewRosterTemplate {
    /// Factions grouped by game system: Vec<(game_system, Vec<faction>)>
    factions_by_system: Vec<(String, Vec<String>)>,
}

pub async fn new_roster_form_handler(State(state): State<AppState>) -> impl IntoResponse {
    let factions_by_system: Vec<(String, Vec<String>)> = state
        .store
        .game_systems()
        .into_iter()
        .map(|gs| {
            let mut facs: Vec<String> = state
                .store
                .all()
                .iter()
                .filter(|ds| ds.game_system == gs)
                .map(|ds| ds.faction.primary.clone())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            facs.sort();
            (gs, facs)
        })
        .collect();

    NewRosterTemplate { factions_by_system }
}

#[derive(Deserialize)]
pub struct NewRosterForm {
    roster_name: String,
    game_system: String,
    faction: String,
}

pub async fn create_roster_handler(
    session: Session,
    Form(form): Form<NewRosterForm>,
) -> impl IntoResponse {
    let roster = RosterList::new(form.roster_name, form.game_system, form.faction);
    save_roster(&session, &roster).await;
    Redirect::to("/roster")
}

// ---------------------------------------------------------------------------
// Roster view templates
// ---------------------------------------------------------------------------

/// Full page (extends base.html) — returned by the initial GET /roster load.
#[derive(Template)]
#[template(path = "roster/view.html")]
struct RosterViewTemplate {
    roster_name: String,
    faction: String,
    game_system: String,
    entries: Vec<RosterEntryViewOwned>,
    total_points: u32,
    has_errors: bool,
    unit_groups: Vec<UnitGroup>,
}

/// Partial (just the #roster-content div) — returned by all HTMX mutation
/// handlers so that the nav/header is never duplicated on swap.
#[derive(Template)]
#[template(path = "roster/_roster_content.html")]
struct RosterContentTemplate {
    roster_name: String,
    faction: String,
    game_system: String,
    entries: Vec<RosterEntryViewOwned>,
    total_points: u32,
    has_errors: bool,
    unit_groups: Vec<UnitGroup>,
}

// ---------------------------------------------------------------------------
// Shared data-building logic
// ---------------------------------------------------------------------------

struct RosterViewData {
    roster_name: String,
    faction: String,
    game_system: String,
    entries: Vec<RosterEntryViewOwned>,
    total_points: u32,
    has_errors: bool,
    unit_groups: Vec<UnitGroup>,
}

fn collect_roster_data(roster: &RosterList, store: &DatasheetStore) -> RosterViewData {
    let entries: Vec<RosterEntryViewOwned> = roster
        .entries
        .iter()
        .filter_map(|entry| {
            store
                .get(&entry.datasheet_id)
                .map(|ds| build_entry_view(entry, ds))
        })
        .collect();

    let total_points: u32 = entries.iter().map(|e| e.points).sum();
    let has_errors = entries
        .iter()
        .any(|e| e.issues.iter().any(|(cls, _)| cls == "error"));

    let unit_groups = build_unit_groups(store, roster);

    RosterViewData {
        roster_name: roster.name.clone(),
        faction: roster.faction.clone(),
        game_system: roster.game_system.clone(),
        entries,
        total_points,
        has_errors,
        unit_groups,
    }
}

fn render_roster_view(roster: &RosterList, store: &DatasheetStore) -> RosterViewTemplate {
    let d = collect_roster_data(roster, store);
    RosterViewTemplate {
        roster_name: d.roster_name,
        faction: d.faction,
        game_system: d.game_system,
        entries: d.entries,
        total_points: d.total_points,
        has_errors: d.has_errors,
        unit_groups: d.unit_groups,
    }
}

fn render_roster_content(roster: &RosterList, store: &DatasheetStore) -> RosterContentTemplate {
    let d = collect_roster_data(roster, store);
    RosterContentTemplate {
        roster_name: d.roster_name,
        faction: d.faction,
        game_system: d.game_system,
        entries: d.entries,
        total_points: d.total_points,
        has_errors: d.has_errors,
        unit_groups: d.unit_groups,
    }
}

pub async fn view_handler(State(state): State<AppState>, session: Session) -> Response {
    let roster = load_roster(&session).await;
    if !roster.is_initialised() {
        return Redirect::to("/roster/new").into_response();
    }
    render_roster_view(&roster, &state.store).into_response()
}

// ---------------------------------------------------------------------------
// Add unit
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AddUnitForm {
    datasheet_id: String,
    /// JSON blob injected by the htmx:configRequest hook — current DOM state of
    /// all configure forms at the moment the Add button was clicked.
    #[serde(default)]
    current_configure: String,
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
    apply_current_configure(&mut roster, &form.current_configure);
    roster.add_unit(&form.datasheet_id, ds.unit_size.min);
    save_roster(&session, &roster).await;

    render_roster_content(&roster, &state.store).into_response()
}

// ---------------------------------------------------------------------------
// Remove unit
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct RemoveUnitForm {
    /// JSON blob injected by the htmx:configRequest hook — current DOM state of
    /// all configure forms at the moment the Remove button was clicked.
    #[serde(default)]
    current_configure: String,
}

pub async fn remove_unit_handler(
    State(state): State<AppState>,
    session: Session,
    Path(entry_id): Path<String>,
    Form(form): Form<RemoveUnitForm>,
) -> impl IntoResponse {
    let mut roster = load_roster(&session).await;
    apply_current_configure(&mut roster, &form.current_configure);
    roster.remove_unit(&entry_id);
    save_roster(&session, &roster).await;
    render_roster_content(&roster, &state.store)
}

// ---------------------------------------------------------------------------
// Reorder unit
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct MoveUnitForm {
    direction: String,
    /// JSON blob injected by the htmx:configRequest hook — current DOM state of
    /// all configure forms at the moment the Move button was clicked.
    #[serde(default)]
    current_configure: String,
}

pub async fn move_unit_handler(
    State(state): State<AppState>,
    session: Session,
    Path(entry_id): Path<String>,
    Form(form): Form<MoveUnitForm>,
) -> impl IntoResponse {
    let mut roster = load_roster(&session).await;
    apply_current_configure(&mut roster, &form.current_configure);
    match form.direction.as_str() {
        "up" => roster.move_unit_up(&entry_id),
        "down" => roster.move_unit_down(&entry_id),
        _ => {}
    }
    save_roster(&session, &roster).await;
    render_roster_content(&roster, &state.store)
}

// ---------------------------------------------------------------------------
// Configure unit (model count + wargear)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct ConfigureUnitForm {
    #[serde(default)]
    model_count: Option<u32>,
    #[serde(default, rename = "options")]
    chosen_options: Vec<String>,
    /// Empty string means "clear custom name".
    #[serde(default)]
    custom_name: String,
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
    entry.custom_name = if form.custom_name.is_empty() {
        None
    } else {
        Some(form.custom_name)
    };

    save_roster(&session, &roster).await;

    // Return the full content partial so the roster header total also updates.
    render_roster_content(&roster, &state.store).into_response()
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
    render_roster_content(&roster, &state.store)
}
