//! Declarative unit validation against a [`UnitDatasheet`].
//!
//! The goal is that *most* game-rule enforcement lives in the JSON schema
//! itself (in `constraints` / `wargear_options`) so the Rust code only needs
//! to interpret those declarative fields — it does not hard-code any
//! game-specific logic.
//!
//! # Quick start
//!
//! ```rust,ignore
//! use adeptus_administratum::datasheet::UnitDatasheet;
//! use adeptus_administratum::validation::{UnitSelection, validate_unit};
//!
//! let datasheet: UnitDatasheet = serde_json::from_str(/* JSON */).unwrap();
//! let selection = UnitSelection {
//!     model_count: 10,
//!     chosen_options: vec!["special_weapon_plasma".into(), "simulacrum_imperialis".into()],
//!     ..Default::default()
//! };
//! let result = validate_unit(&datasheet, &selection);
//! assert!(result.is_ok());
//! ```

use crate::datasheet::{
    ConditionType, Constraint, ConstraintType, Severity, UnitDatasheet,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A user's roster entry for one unit.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnitSelection {
    /// Total number of models in the unit.
    pub model_count: u32,
    /// IDs of wargear options that have been chosen (may repeat for multi-selections).
    pub chosen_options: Vec<String>,
    /// Active unit keywords (base keywords + any added by wargear).
    /// Pre-populate from `UnitDatasheet::keywords` before calling `validate_unit`.
    pub active_keywords: Vec<String>,
    /// IDs of other units already in the same army (for army-unique checks).
    pub army_unit_ids: Vec<String>,
    /// How many times each option ID appears army-wide (for `max_per_army` checks).
    pub army_option_counts: std::collections::HashMap<String, u32>,
}

/// A single validation finding.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    pub constraint_id: String,
    pub severity: Severity,
    pub message: String,
}

/// Errors returned by the validator itself (not constraint violations).
#[derive(Debug, Error)]
pub enum ValidatorError {
    #[error("Weapon ID '{0}' referenced in wargear option '{1}' does not exist on this datasheet")]
    UnknownWeaponRef(String, String),
    #[error("Wargear option '{0}' references unknown option '{1}' in mutually_exclusive_with")]
    UnknownMutualExclusion(String, String),
    #[error("Constraint param '{0}' missing or wrong type in constraint '{1}'")]
    MissingParam(String, String),
}

/// Result of validating a unit.
/// `Ok(issues)` — structure is valid (issues may be empty).
/// `Err(e)`     — the datasheet itself has a structural problem.
pub type ValidationResult = Result<Vec<ValidationIssue>, ValidatorError>;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Validate `selection` against `datasheet`, returning all constraint findings.
///
/// Structural errors in the datasheet (missing weapon refs, etc.) are returned
/// as `Err`.  Constraint violations are returned inside `Ok(issues)`.
pub fn validate_unit(
    datasheet: &UnitDatasheet,
    selection: &UnitSelection,
) -> ValidationResult {
    let mut issues = Vec::new();

    // 1. Structural / referential integrity
    check_wargear_refs(datasheet)?;

    // 2. Unit size
    check_unit_size(datasheet, selection, &mut issues);

    // 3. Wargear option limits
    check_wargear_limits(datasheet, selection, &mut issues);

    // 4. Mutual exclusivity
    check_mutual_exclusion(datasheet, selection, &mut issues);

    // 5. Declarative constraints from the datasheet
    check_declarative_constraints(datasheet, selection, &mut issues)?;

    Ok(issues)
}

// ---------------------------------------------------------------------------
// Step 1 – Structural integrity
// ---------------------------------------------------------------------------

fn check_wargear_refs(datasheet: &UnitDatasheet) -> Result<(), ValidatorError> {
    for opt in &datasheet.wargear_options {
        for w_id in opt.replaces.iter().chain(opt.adds.iter()) {
            if datasheet.weapons.find(w_id).is_none() {
                return Err(ValidatorError::UnknownWeaponRef(
                    w_id.clone(),
                    opt.id.clone(),
                ));
            }
        }
        for excl_id in &opt.mutually_exclusive_with {
            if !datasheet.wargear_options.iter().any(|o| &o.id == excl_id) {
                return Err(ValidatorError::UnknownMutualExclusion(
                    opt.id.clone(),
                    excl_id.clone(),
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Step 2 – Unit size
// ---------------------------------------------------------------------------

fn check_unit_size(
    datasheet: &UnitDatasheet,
    selection: &UnitSelection,
    issues: &mut Vec<ValidationIssue>,
) {
    let count = selection.model_count;
    if !datasheet.unit_size.is_valid_count(count) {
        let legal = legal_sizes_description(datasheet);
        issues.push(ValidationIssue {
            constraint_id: "unit-size".into(),
            severity: Severity::Error,
            message: format!(
                "'{}' has {} model(s), but legal sizes are: {}.",
                datasheet.name, count, legal
            ),
        });
    }
}

fn legal_sizes_description(datasheet: &UnitDatasheet) -> String {
    let sz = &datasheet.unit_size;
    if !sz.fixed_sizes.is_empty() {
        let strs: Vec<_> = sz.fixed_sizes.iter().map(|n| n.to_string()).collect();
        return strs.join(", ");
    }
    if sz.min == sz.max {
        return sz.min.to_string();
    }
    if sz.step == 1 {
        return format!("{}-{}", sz.min, sz.max);
    }
    let mut sizes = vec![];
    let mut n = sz.min;
    while n <= sz.max {
        sizes.push(n.to_string());
        n += sz.step;
    }
    sizes.join(", ")
}

// ---------------------------------------------------------------------------
// Step 3 – Wargear option limits
// ---------------------------------------------------------------------------

fn check_wargear_limits(
    datasheet: &UnitDatasheet,
    selection: &UnitSelection,
    issues: &mut Vec<ValidationIssue>,
) {
    let count_option = |id: &str| selection.chosen_options.iter().filter(|o| o.as_str() == id).count() as u32;

    for opt in &datasheet.wargear_options {
        let times_taken = count_option(&opt.id);
        if times_taken == 0 {
            continue;
        }

        // max_per_unit
        if let Some(max) = opt.limits.max_per_unit {
            if times_taken > max {
                issues.push(ValidationIssue {
                    constraint_id: format!("wargear-max-per-unit-{}", opt.id),
                    severity: Severity::Error,
                    message: format!(
                        "Option '{}' taken {} time(s) but max per unit is {}.",
                        opt.description, times_taken, max
                    ),
                });
            }
        }

        // one_per_n_models
        if let Some(n) = opt.limits.one_per_n_models {
            let allowed = selection.model_count / n;
            if times_taken > allowed {
                issues.push(ValidationIssue {
                    constraint_id: format!("wargear-per-n-{}", opt.id),
                    severity: Severity::Error,
                    message: format!(
                        "Option '{}' allows 1 per {} models; unit has {} model(s) so max is {}, but {} taken.",
                        opt.description, n, selection.model_count, allowed, times_taken
                    ),
                });
            }
        }

        // requires_min_unit_size
        if let Some(min_size) = opt.limits.requires_min_unit_size {
            if selection.model_count < min_size {
                issues.push(ValidationIssue {
                    constraint_id: format!("wargear-min-size-{}", opt.id),
                    severity: Severity::Error,
                    message: format!(
                        "Option '{}' requires at least {} models; unit has {}.",
                        opt.description, min_size, selection.model_count
                    ),
                });
            }
        }

        // max_per_army
        if let Some(army_max) = opt.limits.max_per_army {
            let army_count = selection
                .army_option_counts
                .get(&opt.id)
                .copied()
                .unwrap_or(0);
            if army_count > army_max {
                issues.push(ValidationIssue {
                    constraint_id: format!("wargear-max-per-army-{}", opt.id),
                    severity: Severity::Error,
                    message: format!(
                        "Option '{}' appears {} time(s) in the army but army max is {}.",
                        opt.description, army_count, army_max
                    ),
                });
            }
        }

        // is_unique (army-wide)
        if opt.is_unique {
            let army_count = selection
                .army_option_counts
                .get(&opt.id)
                .copied()
                .unwrap_or(0);
            if army_count > 1 {
                issues.push(ValidationIssue {
                    constraint_id: format!("wargear-unique-{}", opt.id),
                    severity: Severity::Error,
                    message: format!(
                        "'{}' is unique but appears {} time(s) in the army.",
                        opt.description, army_count
                    ),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Step 4 – Mutual exclusivity
// ---------------------------------------------------------------------------

fn check_mutual_exclusion(
    datasheet: &UnitDatasheet,
    selection: &UnitSelection,
    issues: &mut Vec<ValidationIssue>,
) {
    // We only check each pair once (A conflicts with B is the same as B conflicts with A).
    let mut checked = std::collections::HashSet::new();

    for opt in &datasheet.wargear_options {
        if !selection.chosen_options.contains(&opt.id) {
            continue;
        }
        for excl_id in &opt.mutually_exclusive_with {
            if !selection.chosen_options.contains(excl_id) {
                continue;
            }
            let key = if opt.id < *excl_id {
                (opt.id.clone(), excl_id.clone())
            } else {
                (excl_id.clone(), opt.id.clone())
            };
            if checked.insert(key) {
                let excl_opt = datasheet
                    .wargear_options
                    .iter()
                    .find(|o| &o.id == excl_id)
                    .map(|o| o.description.as_str())
                    .unwrap_or(excl_id.as_str());
                issues.push(ValidationIssue {
                    constraint_id: format!("mutual-excl-{}-{}", opt.id, excl_id),
                    severity: Severity::Error,
                    message: format!(
                        "Options '{}' and '{}' are mutually exclusive.",
                        opt.description, excl_opt
                    ),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Step 5 – Declarative constraints
// ---------------------------------------------------------------------------

fn check_declarative_constraints(
    datasheet: &UnitDatasheet,
    selection: &UnitSelection,
    issues: &mut Vec<ValidationIssue>,
) -> Result<(), ValidatorError> {
    for constraint in &datasheet.constraints {
        // Skip if the constraint's own condition is not met.
        if let Some(cond) = &constraint.condition {
            if !evaluate_condition(cond, datasheet, selection) {
                continue;
            }
        }

        match constraint.constraint_type {
            ConstraintType::UnitSize => check_constraint_unit_size(constraint, datasheet, selection, issues)?,
            ConstraintType::WargearLimit => check_constraint_wargear_limit(constraint, selection, issues)?,
            ConstraintType::ArmyUnique => check_constraint_army_unique(constraint, selection, issues),
            ConstraintType::ModelCountPerWargear => {
                check_constraint_model_count_per_wargear(constraint, datasheet, selection, issues)?
            }
            // Other types are extensible; skip unknown for now.
            _ => {}
        }
    }
    Ok(())
}

// --- unit_size constraint ---

fn check_constraint_unit_size(
    constraint: &Constraint,
    datasheet: &UnitDatasheet,
    selection: &UnitSelection,
    issues: &mut Vec<ValidationIssue>,
) -> Result<(), ValidatorError> {
    // `allowed_sizes` overrides unit_size.fixed_sizes from schema
    if let Some(allowed) = constraint.params.get("allowed_sizes") {
        if let Some(arr) = allowed.as_array() {
            let allowed_counts: Vec<u32> = arr
                .iter()
                .filter_map(|v| v.as_u64().map(|n| n as u32))
                .collect();
            if !allowed_counts.contains(&selection.model_count) {
                let list: Vec<_> = allowed_counts.iter().map(|n| n.to_string()).collect();
                push_constraint_issue(
                    constraint,
                    &format!(
                        "{} — unit has {} model(s), allowed: [{}].",
                        constraint.description,
                        selection.model_count,
                        list.join(", ")
                    ),
                    issues,
                );
            }
            return Ok(());
        }
        return Err(ValidatorError::MissingParam(
            "allowed_sizes".into(),
            constraint.id.clone(),
        ));
    }
    // Fall back to the datasheet's unit_size definition.
    if !datasheet.unit_size.is_valid_count(selection.model_count) {
        push_constraint_issue(constraint, &constraint.description, issues);
    }
    Ok(())
}

// --- wargear_limit constraint ---

fn check_constraint_wargear_limit(
    constraint: &Constraint,
    selection: &UnitSelection,
    issues: &mut Vec<ValidationIssue>,
) -> Result<(), ValidatorError> {
    let option_id = constraint
        .params
        .get("option_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ValidatorError::MissingParam("option_id".into(), constraint.id.clone()))?;
    let max = constraint
        .params
        .get("max")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ValidatorError::MissingParam("max".into(), constraint.id.clone()))?;

    let count = selection
        .chosen_options
        .iter()
        .filter(|o| o.as_str() == option_id)
        .count() as u64;

    if count > max {
        push_constraint_issue(
            constraint,
            &format!(
                "{} — taken {} time(s), max is {}.",
                constraint.description, count, max
            ),
            issues,
        );
    }
    Ok(())
}

// --- army_unique constraint ---

fn check_constraint_army_unique(
    constraint: &Constraint,
    selection: &UnitSelection,
    issues: &mut Vec<ValidationIssue>,
) {
    if let Some(unit_id) = constraint.params.get("unit_id").and_then(|v| v.as_str()) {
        let count = selection
            .army_unit_ids
            .iter()
            .filter(|id| id.as_str() == unit_id)
            .count();
        if count > 1 {
            push_constraint_issue(
                constraint,
                &format!(
                    "{} — '{}' appears {} time(s) in the army but must be unique.",
                    constraint.description, unit_id, count
                ),
                issues,
            );
        }
    }
}

// --- model_count_per_wargear constraint ---

fn check_constraint_model_count_per_wargear(
    constraint: &Constraint,
    _datasheet: &UnitDatasheet,
    selection: &UnitSelection,
    issues: &mut Vec<ValidationIssue>,
) -> Result<(), ValidatorError> {
    // Expects params: { "option_ids": [...], "count_method": "sum_selections", "limit_formula": "floor(unit_size / 5)" }
    let option_ids = constraint
        .params
        .get("option_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            ValidatorError::MissingParam("option_ids".into(), constraint.id.clone())
        })?;

    let total_selected: u32 = option_ids
        .iter()
        .filter_map(|v| v.as_str())
        .map(|id| {
            selection
                .chosen_options
                .iter()
                .filter(|o| o.as_str() == id)
                .count() as u32
        })
        .sum();

    // Parse simple "floor(unit_size / N)" formula from params.
    let limit = if let Some(formula) = constraint
        .params
        .get("limit_formula")
        .and_then(|v| v.as_str())
    {
        parse_floor_unit_size_formula(formula, selection.model_count).ok_or_else(|| {
            ValidatorError::MissingParam("limit_formula".into(), constraint.id.clone())
        })?
    } else {
        return Err(ValidatorError::MissingParam(
            "limit_formula".into(),
            constraint.id.clone(),
        ));
    };

    if total_selected > limit {
        push_constraint_issue(
            constraint,
            &format!(
                "{} — {} selected, but limit for {} model(s) is {}.",
                constraint.description, total_selected, selection.model_count, limit
            ),
            issues,
        );
    }
    Ok(())
}

/// Parse `"floor(unit_size / N)"` and return the result.
fn parse_floor_unit_size_formula(formula: &str, unit_size: u32) -> Option<u32> {
    // Very simple parser: only handles the pattern "floor(unit_size / <int>)"
    let s = formula.trim();
    let inner = s
        .strip_prefix("floor(unit_size / ")
        .and_then(|s| s.strip_suffix(')'))?;
    let divisor: u32 = inner.trim().parse().ok()?;
    Some(unit_size / divisor)
}

// ---------------------------------------------------------------------------
// Condition evaluator
// ---------------------------------------------------------------------------

fn evaluate_condition(
    cond: &crate::datasheet::Condition,
    datasheet: &UnitDatasheet,
    selection: &UnitSelection,
) -> bool {
    match cond.condition_type {
        ConditionType::HasKeyword => cond
            .keyword
            .as_deref()
            .map(|kw| selection.active_keywords.iter().any(|k| k == kw))
            .unwrap_or(false),

        ConditionType::HasWargearOption => cond
            .option_id
            .as_deref()
            .map(|id| selection.chosen_options.contains(&id.to_string()))
            .unwrap_or(false),

        ConditionType::UnitSizeGte => cond
            .value
            .map(|v| selection.model_count >= v as u32)
            .unwrap_or(false),

        ConditionType::UnitSizeLte => cond
            .value
            .map(|v| selection.model_count <= v as u32)
            .unwrap_or(false),

        ConditionType::ArmyIncludesUnit => cond
            .unit_id
            .as_deref()
            .map(|id| selection.army_unit_ids.contains(&id.to_string()))
            .unwrap_or(false),

        ConditionType::AllOf => cond
            .conditions
            .iter()
            .all(|c| evaluate_condition(c, datasheet, selection)),

        ConditionType::AnyOf => cond
            .conditions
            .iter()
            .any(|c| evaluate_condition(c, datasheet, selection)),

        ConditionType::NoneOf => !cond
            .conditions
            .iter()
            .any(|c| evaluate_condition(c, datasheet, selection)),

        // HasAbility requires scanning abilities; simplified here.
        ConditionType::HasAbility => cond
            .ability_name
            .as_deref()
            .map(|name| datasheet.abilities.iter().any(|a| a.name == name))
            .unwrap_or(false),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn push_constraint_issue(
    constraint: &Constraint,
    message: &str,
    issues: &mut Vec<ValidationIssue>,
) {
    issues.push(ValidationIssue {
        constraint_id: constraint.id.clone(),
        severity: constraint.severity.clone(),
        message: message.to_string(),
    });
}

// ---------------------------------------------------------------------------
// Points calculation
// ---------------------------------------------------------------------------

/// Calculate total points for a selection.
pub fn calculate_points(datasheet: &UnitDatasheet, selection: &UnitSelection) -> u32 {
    let mut total = datasheet.points.base;

    // Per-additional-model cost (beyond the minimum unit size).
    if let Some(per_model) = datasheet.points.per_additional_model {
        let extra = selection.model_count.saturating_sub(datasheet.unit_size.min);
        total += extra * per_model;
    }

    // Wargear option costs.
    for opt_id in &selection.chosen_options {
        if let Some(opt) = datasheet.wargear_options.iter().find(|o| &o.id == opt_id) {
            let cost = match opt.cost_per {
                crate::datasheet::CostPer::Selection => opt.points_cost,
                crate::datasheet::CostPer::Model => opt.points_cost * selection.model_count as i32,
            };
            // Cost can be negative (some options reduce points).
            total = (total as i32 + cost).max(0) as u32;
        }
    }

    total
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datasheet::UnitDatasheet;

    fn load_battle_sisters() -> UnitDatasheet {
        let json = include_str!("../data/40k/adepta-sororitas/battle-sisters-squad.json");
        serde_json::from_str(json).expect("Failed to parse battle-sisters-squad.json")
    }

    fn base_selection(model_count: u32) -> UnitSelection {
        UnitSelection {
            model_count,
            chosen_options: vec![],
            active_keywords: vec![
                "INFANTRY".into(),
                "BATTLELINE".into(),
                "ADEPTA SORORITAS".into(),
            ],
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // Unit size
    // -----------------------------------------------------------------------

    #[test]
    fn valid_unit_size_5() {
        let ds = load_battle_sisters();
        let sel = base_selection(5);
        let result = validate_unit(&ds, &sel).expect("Validator error");
        assert!(result.is_empty(), "Expected no issues: {:?}", result);
    }

    #[test]
    fn valid_unit_size_10() {
        let ds = load_battle_sisters();
        let sel = base_selection(10);
        let result = validate_unit(&ds, &sel).expect("Validator error");
        assert!(result.is_empty(), "Expected no issues: {:?}", result);
    }

    #[test]
    fn invalid_unit_size_7() {
        let ds = load_battle_sisters();
        let sel = base_selection(7);
        let issues = validate_unit(&ds, &sel).expect("Validator error");
        assert!(
            issues.iter().any(|i| i.constraint_id == "unit-size-valid"),
            "Expected unit-size-valid constraint to fire"
        );
    }

    #[test]
    fn invalid_unit_size_0() {
        let ds = load_battle_sisters();
        let sel = base_selection(0);
        let issues = validate_unit(&ds, &sel).expect("Validator error");
        // Both the generic unit-size check and the declarative constraint fire.
        assert!(!issues.is_empty(), "Expected at least one issue");
    }

    // -----------------------------------------------------------------------
    // Special weapons per-5-model rule
    // -----------------------------------------------------------------------

    #[test]
    fn one_special_weapon_in_5_man_unit_is_valid() {
        let ds = load_battle_sisters();
        let mut sel = base_selection(5);
        sel.chosen_options = vec!["special_weapon_plasma".into()];
        let issues = validate_unit(&ds, &sel).expect("Validator error");
        assert!(issues.is_empty(), "Expected no issues: {:?}", issues);
    }

    #[test]
    fn two_special_weapons_in_5_man_unit_is_invalid() {
        let ds = load_battle_sisters();
        let mut sel = base_selection(5);
        sel.chosen_options = vec![
            "special_weapon_plasma".into(),
            "special_weapon_plasma".into(),
        ];
        let issues = validate_unit(&ds, &sel).expect("Validator error");
        assert!(
            issues
                .iter()
                .any(|i| i.constraint_id == "special-weapons-per-five"),
            "Expected special-weapons-per-five to fire; got {:?}",
            issues
        );
    }

    #[test]
    fn two_special_weapons_in_10_man_unit_is_valid() {
        let ds = load_battle_sisters();
        let mut sel = base_selection(10);
        sel.chosen_options = vec![
            "special_weapon_melta".into(),
            "special_weapon_melta".into(),
        ];
        let issues = validate_unit(&ds, &sel).expect("Validator error");
        assert!(issues.is_empty(), "Expected no issues: {:?}", issues);
    }

    // -----------------------------------------------------------------------
    // Mutual exclusivity
    // -----------------------------------------------------------------------

    #[test]
    fn mixing_special_weapon_types_is_invalid() {
        let ds = load_battle_sisters();
        let mut sel = base_selection(10);
        sel.chosen_options = vec![
            "special_weapon_plasma".into(),
            "special_weapon_melta".into(),
        ];
        let issues = validate_unit(&ds, &sel).expect("Validator error");
        assert!(
            issues.iter().any(|i| i.constraint_id.starts_with("mutual-excl-")),
            "Expected mutual exclusion issue; got {:?}",
            issues
        );
    }

    // -----------------------------------------------------------------------
    // Simulacrum Imperialis
    // -----------------------------------------------------------------------

    #[test]
    fn simulacrum_is_valid_once() {
        let ds = load_battle_sisters();
        let mut sel = base_selection(5);
        sel.chosen_options = vec!["simulacrum_imperialis".into()];
        let issues = validate_unit(&ds, &sel).expect("Validator error");
        assert!(issues.is_empty(), "Expected no issues: {:?}", issues);
    }

    #[test]
    fn simulacrum_twice_is_invalid() {
        let ds = load_battle_sisters();
        let mut sel = base_selection(10);
        sel.chosen_options = vec![
            "simulacrum_imperialis".into(),
            "simulacrum_imperialis".into(),
        ];
        let issues = validate_unit(&ds, &sel).expect("Validator error");
        assert!(
            issues
                .iter()
                .any(|i| i.constraint_id == "simulacrum-one-per-unit"),
            "Expected simulacrum-one-per-unit to fire; got {:?}",
            issues
        );
    }

    // -----------------------------------------------------------------------
    // Points
    // -----------------------------------------------------------------------

    #[test]
    fn base_points_5_no_options() {
        let ds = load_battle_sisters();
        let sel = base_selection(5);
        assert_eq!(calculate_points(&ds, &sel), 90);
    }

    #[test]
    fn power_weapon_upgrade_adds_5_pts() {
        let ds = load_battle_sisters();
        let mut sel = base_selection(5);
        sel.chosen_options = vec!["sister_superior_power_weapon".into()];
        assert_eq!(calculate_points(&ds, &sel), 95);
    }

    #[test]
    fn simulacrum_adds_5_pts() {
        let ds = load_battle_sisters();
        let mut sel = base_selection(5);
        sel.chosen_options = vec!["simulacrum_imperialis".into()];
        assert_eq!(calculate_points(&ds, &sel), 95);
    }

    #[test]
    fn combined_upgrades_stack() {
        let ds = load_battle_sisters();
        let mut sel = base_selection(5);
        sel.chosen_options = vec![
            "sister_superior_power_weapon".into(),
            "simulacrum_imperialis".into(),
        ];
        assert_eq!(calculate_points(&ds, &sel), 100);
    }
}
