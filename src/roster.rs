//! Army roster — the user's list of selected units with their wargear choices.

use crate::store::DatasheetStore;
use crate::validation::{calculate_points, validate_unit, UnitSelection, ValidationIssue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One unit entry within a roster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterEntry {
    /// Stable UUID string assigned when the unit is added.
    pub entry_id: String,
    /// References a [`UnitDatasheet::id`] in the store.
    pub datasheet_id: String,
    /// The user's current selections for this entry.
    pub selection: UnitSelection,
}

/// An army roster (the user's list being built).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RosterList {
    pub name: String,
    pub entries: Vec<RosterEntry>,
}

/// Per-entry validation result — pairs the entry ID with any issues found.
#[derive(Debug, Clone)]
pub struct EntryValidation {
    pub entry_id: String,
    pub datasheet_name: String,
    pub issues: Vec<ValidationIssue>,
    pub points: u32,
}

impl RosterList {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: Vec::new(),
        }
    }

    pub fn add_unit(&mut self, datasheet_id: impl Into<String>, model_count: u32) -> String {
        let entry_id = uuid::Uuid::new_v4().to_string();
        self.entries.push(RosterEntry {
            entry_id: entry_id.clone(),
            datasheet_id: datasheet_id.into(),
            selection: UnitSelection {
                model_count,
                ..Default::default()
            },
        });
        entry_id
    }

    pub fn remove_unit(&mut self, entry_id: &str) {
        self.entries.retain(|e| e.entry_id != entry_id);
    }

    pub fn get_entry_mut(&mut self, entry_id: &str) -> Option<&mut RosterEntry> {
        self.entries.iter_mut().find(|e| e.entry_id == entry_id)
    }

    /// Total points across all entries, using current wargear selections.
    pub fn total_points(&self, store: &DatasheetStore) -> u32 {
        self.entries
            .iter()
            .filter_map(|entry| {
                store
                    .get(&entry.datasheet_id)
                    .map(|ds| calculate_points(ds, &entry.selection))
            })
            .sum()
    }

    /// Validate every entry, populating army-wide context (option counts, unit IDs)
    /// so cross-unit constraints (max_per_army, army_unique) work correctly.
    pub fn validate_army(&self, store: &DatasheetStore) -> Vec<EntryValidation> {
        // Build army-wide aggregate data for cross-unit constraints.
        let army_unit_ids: Vec<String> = self
            .entries
            .iter()
            .map(|e| e.datasheet_id.clone())
            .collect();

        let mut army_option_counts: HashMap<String, u32> = HashMap::new();
        for entry in &self.entries {
            for opt_id in &entry.selection.chosen_options {
                *army_option_counts.entry(opt_id.clone()).or_default() += 1;
            }
        }

        self.entries
            .iter()
            .filter_map(|entry| {
                let ds = store.get(&entry.datasheet_id)?;

                // Inject army-wide context into this entry's selection.
                let mut sel = entry.selection.clone();
                sel.army_unit_ids = army_unit_ids.clone();
                sel.army_option_counts = army_option_counts.clone();

                // Seed active keywords from the datasheet.
                if sel.active_keywords.is_empty() {
                    sel.active_keywords = ds
                        .keywords
                        .unit
                        .iter()
                        .chain(ds.keywords.faction.iter())
                        .cloned()
                        .collect();
                }

                let issues = validate_unit(ds, &sel).unwrap_or_default();
                let points = calculate_points(ds, &sel);

                Some(EntryValidation {
                    entry_id: entry.entry_id.clone(),
                    datasheet_name: ds.name.clone(),
                    issues,
                    points,
                })
            })
            .collect()
    }

    pub fn has_errors(&self, store: &DatasheetStore) -> bool {
        self.validate_army(store)
            .iter()
            .any(|ev| ev.issues.iter().any(|i| matches!(i.severity, crate::datasheet::Severity::Error)))
    }
}
