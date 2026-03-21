use crate::datasheet::{ProfileCount, Severity, TransportRule, UnitDatasheet, WargearOptionType};
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

/// One weapon slot inside a weapon group (either the base weapon or one replacement option).
struct WeaponSlotView {
    weapon_name: String,
    /// Current count — for the base slot this is auto-computed; for option slots it's chosen qty.
    count: i32,
    /// Empty string = this is the base/default weapon (read-only display).
    option_id: String,
    /// Stored on the base slot as `data-base-max` so JS can recalculate the display live.
    /// For option slots this is the per-unit max qty at the current model count.
    max_qty: u32,
    /// Comma-separated option IDs mutually exclusive with this slot (for JS live-exclusion).
    mutually_exclusive_ids: String,
}

/// A group of weapon slots that share the same "base" weapon being replaced.
struct WeaponGroupView {
    slots: Vec<WeaponSlotView>,
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
    // ── Linking ────────────────────────────────────────────────────────────
    /// Display name of the CHARACTER currently leading this unit. Empty = none assigned.
    assigned_leader_name: String,
    /// Display name of the TRANSPORT this unit is currently embarked in. Empty = none assigned.
    assigned_transport_name: String,
    /// Other roster entries (entry_id, display_name) that are eligible to lead this unit.
    available_leaders: Vec<(String, String)>,
    /// Other roster entries (entry_id, display_name) whose transport can carry this unit.
    available_transports: Vec<(String, String)>,
    /// Display names of units this entry is currently leading (CHARACTER → squads).
    units_leading: Vec<String>,
    /// Display names of units this entry is currently carrying (TRANSPORT → passengers).
    units_transporting: Vec<String>,
    /// Weapon groups for qty-mode weapon-swap options (shown as number spinners).
    weapon_groups: Vec<WeaponGroupView>,
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

fn display_name(entry: &RosterEntry, ds: &UnitDatasheet) -> String {
    entry.custom_name.clone().unwrap_or_else(|| ds.name.clone())
}

/// Returns `true` if a unit with the given keywords can board `transport_ds`.
fn can_board_transport(transport_ds: &UnitDatasheet, unit_keywords: &[String]) -> bool {
    let transport = match &transport_ds.transport {
        Some(t) => t,
        None => return false,
    };
    for restriction in &transport.restrictions {
        match restriction.rule {
            TransportRule::MustHaveKeyword => {
                if let Some(kw) = &restriction.keyword {
                    if !unit_keywords.iter().any(|k| k.eq_ignore_ascii_case(kw)) {
                        return false;
                    }
                }
            }
            TransportRule::MustNotHaveKeyword => {
                if let Some(kw) = &restriction.keyword {
                    if unit_keywords.iter().any(|k| k.eq_ignore_ascii_case(kw)) {
                        return false;
                    }
                }
            }
            _ => {}
        }
    }
    true
}

/// Maximum number of times a wargear option may be taken at the current model count.
fn compute_max_qty(opt: &crate::datasheet::WargearOption, model_count: u32) -> u32 {
    if let Some(n) = opt.limits.one_per_n_models {
        model_count / n
    } else if let Some(max) = opt.limits.max_per_unit {
        max
    } else {
        1
    }
}

/// Build weapon groups for qty-mode replace options (those that can be taken by >1 model).
///
/// Returns one `WeaponGroupView` per distinct set of replaced weapons.  Each group
/// contains a read-only base-weapon slot followed by one editable slot per option.
fn build_weapon_groups(
    ds: &UnitDatasheet,
    sel: &crate::validation::UnitSelection,
) -> Vec<WeaponGroupView> {
    use std::collections::HashMap;

    // Collect Replace-type options that are available at the current model count
    // and replace exactly one weapon (multi-weapon swaps have ambiguous base
    // counts and stay as checkboxes).
    let qty_opts: Vec<&crate::datasheet::WargearOption> = ds
        .wargear_options
        .iter()
        .filter(|opt| {
            opt.option_type == WargearOptionType::Replace
                && opt.replaces.len() == 1
                && compute_max_qty(opt, sel.model_count) >= 1
        })
        .collect();

    if qty_opts.is_empty() {
        return vec![];
    }

    // Resolve effective model count for each profile (Fixed vs Remainder).
    let fixed_total: u32 = ds
        .profiles
        .iter()
        .filter_map(|p| match &p.count {
            Some(ProfileCount::Fixed(n)) => Some(*n),
            _ => None,
        })
        .sum();

    let profile_model_count = |profile_name: &str| -> u32 {
        ds.profiles
            .iter()
            .find(|p| p.name == profile_name)
            .map(|p| match &p.count {
                Some(ProfileCount::Fixed(n)) => *n,
                Some(ProfileCount::Remainder(_)) => sel.model_count.saturating_sub(fixed_total),
                None => sel.model_count,
            })
            .unwrap_or(0)
    };

    // Group options by their sorted replaces-key, maintaining JSON order.
    let mut group_keys: Vec<String> = Vec::new();
    let mut groups_map: HashMap<String, Vec<&crate::datasheet::WargearOption>> = HashMap::new();
    for opt in &qty_opts {
        let mut key = opt.replaces.clone();
        key.sort();
        let key_str = key.join(",");
        if !groups_map.contains_key(&key_str) {
            group_keys.push(key_str.clone());
        }
        groups_map.entry(key_str).or_default().push(opt);
    }

    let mut result = Vec::new();

    for key_str in group_keys {
        let opts = &groups_map[&key_str];
        let replaced_ids: Vec<&str> = key_str.split(',').collect();

        // Count how many models carry the replaced weapon(s) in the default loadout.
        let base_count: i32 = ds
            .default_loadout
            .iter()
            .filter(|e| replaced_ids.contains(&e.weapon_id.as_str()))
            .map(|e| {
                let models: u32 = if let Some(at) = &e.applies_to {
                    if let Some(pname) = &at.profile_name {
                        profile_model_count(pname)
                    } else if let Some(mc) = at.model_count {
                        mc
                    } else {
                        sel.model_count
                    }
                } else {
                    sel.model_count
                };
                models as i32 * e.quantity as i32
            })
            .sum();

        if base_count == 0 {
            continue; // Can't determine base count — skip this group.
        }

        // Sum of all chosen quantities across options in this group.
        let total_chosen: i32 = opts
            .iter()
            .map(|opt| {
                sel.chosen_options
                    .iter()
                    .filter(|o| o.as_str() == opt.id)
                    .count() as i32
            })
            .sum();

        let base_name = replaced_ids
            .first()
            .and_then(|id| ds.weapons.find(id).map(|w| w.name.clone()))
            .unwrap_or_default();

        let mut slots = Vec::new();

        // First slot: the base/default weapon (read-only display).
        slots.push(WeaponSlotView {
            weapon_name: base_name,
            count: (base_count - total_chosen).max(0),
            option_id: String::new(),
            max_qty: base_count as u32, // used as data-base-max for JS live-recalculation
            mutually_exclusive_ids: String::new(),
        });

        // Remaining slots: one per option (in JSON order).
        for opt in opts.iter() {
            let current_qty = sel
                .chosen_options
                .iter()
                .filter(|o| o.as_str() == opt.id)
                .count() as i32;
            let max_qty = compute_max_qty(opt, sel.model_count);
            let weapon_name = opt
                .adds
                .first()
                .and_then(|id| ds.weapons.find(id).map(|w| w.name.clone()))
                .unwrap_or_else(|| opt.description.clone());

            slots.push(WeaponSlotView {
                weapon_name,
                count: current_qty,
                option_id: opt.id.clone(),
                max_qty,
                mutually_exclusive_ids: opt.mutually_exclusive_with.join(","),
            });
        }

        result.push(WeaponGroupView { slots });
    }

    result
}

fn build_entry_view(
    entry: &RosterEntry,
    ds: &UnitDatasheet,
    all_pairs: &[(&RosterEntry, &UnitDatasheet)],
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

    // Options shown as quantity spinners in the weapon breakdown — exclude from checkboxes.
    let qty_mode_ids: std::collections::HashSet<String> = ds
        .wargear_options
        .iter()
        .filter(|opt| {
            opt.option_type == WargearOptionType::Replace
                && opt.replaces.len() == 1
                && compute_max_qty(opt, sel.model_count) >= 1
        })
        .map(|opt| opt.id.clone())
        .collect();

    let options = ds
        .wargear_options
        .iter()
        .filter(|opt| !qty_mode_ids.contains(&opt.id))
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

    // All keywords for this unit (unit + faction).
    let unit_keywords: Vec<String> = ds
        .keywords
        .unit
        .iter()
        .chain(ds.keywords.faction.iter())
        .cloned()
        .collect();

    // Available leaders: other roster entries whose Leader.can_lead includes this datasheet.
    let available_leaders: Vec<(String, String)> = all_pairs
        .iter()
        .filter(|(other_entry, other_ds)| {
            other_entry.entry_id != entry.entry_id
                && other_ds
                    .leader
                    .as_ref()
                    .map_or(false, |l| l.can_lead.contains(&entry.datasheet_id))
                // Don't offer a leader that is already leading another unit.
                && !all_pairs.iter().any(|(e, _)| {
                    e.entry_id != entry.entry_id
                        && e.assigned_leader_entry_id.as_deref()
                            == Some(&other_entry.entry_id)
                })
        })
        .map(|(other_entry, other_ds)| {
            (other_entry.entry_id.clone(), display_name(other_entry, other_ds))
        })
        .collect();

    // Available transports: other roster entries that are transports and can carry this unit.
    let available_transports: Vec<(String, String)> = all_pairs
        .iter()
        .filter(|(other_entry, other_ds)| {
            other_entry.entry_id != entry.entry_id
                && can_board_transport(other_ds, &unit_keywords)
        })
        .map(|(other_entry, other_ds)| {
            (other_entry.entry_id.clone(), display_name(other_entry, other_ds))
        })
        .collect();

    // Resolved assigned leader name (empty string = none assigned).
    let assigned_leader_name = entry
        .assigned_leader_entry_id
        .as_ref()
        .and_then(|lid| {
            all_pairs
                .iter()
                .find(|(e, _)| &e.entry_id == lid)
                .map(|(e, ds)| display_name(e, ds))
        })
        .unwrap_or_default();

    // Resolved assigned transport name (empty string = none assigned).
    let assigned_transport_name = entry
        .assigned_transport_entry_id
        .as_ref()
        .and_then(|tid| {
            all_pairs
                .iter()
                .find(|(e, _)| &e.entry_id == tid)
                .map(|(e, ds)| display_name(e, ds))
        })
        .unwrap_or_default();

    // Units that have selected this entry as their leader.
    let units_leading: Vec<String> = all_pairs
        .iter()
        .filter(|(other_entry, _)| {
            other_entry.assigned_leader_entry_id.as_deref() == Some(&entry.entry_id)
        })
        .map(|(other_entry, other_ds)| display_name(other_entry, other_ds))
        .collect();

    // Units that have selected this entry as their transport.
    let units_transporting: Vec<String> = all_pairs
        .iter()
        .filter(|(other_entry, _)| {
            other_entry.assigned_transport_entry_id.as_deref() == Some(&entry.entry_id)
        })
        .map(|(other_entry, other_ds)| display_name(other_entry, other_ds))
        .collect();

    let weapon_groups = build_weapon_groups(ds, &sel);

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
        assigned_leader_name,
        assigned_transport_name,
        available_leaders,
        available_transports,
        units_leading,
        units_transporting,
        weapon_groups,
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
    // Resolve all (entry, datasheet) pairs up-front so linking logic can cross-reference.
    let all_pairs: Vec<(&RosterEntry, &UnitDatasheet)> = roster
        .entries
        .iter()
        .filter_map(|e| store.get(&e.datasheet_id).map(|ds| (e, ds)))
        .collect();

    let entries: Vec<RosterEntryViewOwned> = all_pairs
        .iter()
        .map(|(entry, ds)| build_entry_view(entry, ds, &all_pairs))
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

// ---------------------------------------------------------------------------
// Assign leader
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AssignLeaderForm {
    /// Entry ID of the leader to attach, or empty string to unlink.
    #[serde(default)]
    leader_entry_id: String,
    #[serde(default)]
    current_configure: String,
}

pub async fn assign_leader_handler(
    State(state): State<AppState>,
    session: Session,
    Path(entry_id): Path<String>,
    Form(form): Form<AssignLeaderForm>,
) -> impl IntoResponse {
    let mut roster = load_roster(&session).await;
    apply_current_configure(&mut roster, &form.current_configure);
    let leader_id = if form.leader_entry_id.is_empty() {
        None
    } else {
        Some(form.leader_entry_id.as_str())
    };
    roster.assign_leader(&entry_id, leader_id);
    save_roster(&session, &roster).await;
    render_roster_content(&roster, &state.store)
}

// ---------------------------------------------------------------------------
// Assign transport
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AssignTransportForm {
    /// Entry ID of the transport to embark into, or empty string to unlink.
    #[serde(default)]
    transport_entry_id: String,
    #[serde(default)]
    current_configure: String,
}

pub async fn assign_transport_handler(
    State(state): State<AppState>,
    session: Session,
    Path(entry_id): Path<String>,
    Form(form): Form<AssignTransportForm>,
) -> impl IntoResponse {
    let mut roster = load_roster(&session).await;
    apply_current_configure(&mut roster, &form.current_configure);
    let transport_id = if form.transport_entry_id.is_empty() {
        None
    } else {
        Some(form.transport_entry_id.as_str())
    };
    roster.assign_transport(&entry_id, transport_id);
    save_roster(&session, &roster).await;
    render_roster_content(&roster, &state.store)
}
