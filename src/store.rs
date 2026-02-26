//! Runtime datasheet store.
//!
//! Scans the `data/` directory tree at startup, parses every `*.json` file as
//! a [`UnitDatasheet`], and indexes them by their `id` field.  Wrapped in
//! `Arc<DatasheetStore>` so it can be shared across Axum handlers.

use crate::datasheet::UnitDatasheet;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug)]
pub struct DatasheetStore {
    by_id: HashMap<String, UnitDatasheet>,
}

impl DatasheetStore {
    /// Load every `*.json` file under `dir` as a [`UnitDatasheet`].
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        let mut by_id = HashMap::new();

        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            let ds: UnitDatasheet = serde_json::from_str(&raw)
                .with_context(|| format!("parsing {}", path.display()))?;
            by_id.insert(ds.id.clone(), ds);
        }

        Ok(Self { by_id })
    }

    pub fn get(&self, id: &str) -> Option<&UnitDatasheet> {
        self.by_id.get(id)
    }

    pub fn all(&self) -> Vec<&UnitDatasheet> {
        let mut v: Vec<_> = self.by_id.values().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn by_faction(&self, faction: &str) -> Vec<&UnitDatasheet> {
        let mut v: Vec<_> = self
            .by_id
            .values()
            .filter(|ds| ds.faction.primary.eq_ignore_ascii_case(faction))
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn factions(&self) -> Vec<String> {
        let mut factions: Vec<String> = self
            .by_id
            .values()
            .map(|ds| ds.faction.primary.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        factions.sort();
        factions
    }

    /// Returns all units valid for a roster with the given game system and primary faction.
    /// Includes the primary faction's units plus any units from factions listed as allies.
    pub fn units_for_roster<'a>(&'a self, game_system: &str, faction: &str) -> Vec<&'a UnitDatasheet> {
        // Collect allied faction names from primary-faction units.
        let allied: std::collections::HashSet<&str> = self
            .by_id
            .values()
            .filter(|ds| ds.game_system == game_system && ds.faction.primary == faction)
            .flat_map(|ds| ds.faction.allies.iter().map(String::as_str))
            .collect();

        let mut v: Vec<_> = self
            .by_id
            .values()
            .filter(|ds| {
                ds.game_system == game_system
                    && (ds.faction.primary == faction
                        || allied.contains(ds.faction.primary.as_str()))
            })
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn game_systems(&self) -> Vec<String> {
        let mut systems: Vec<String> = self
            .by_id
            .values()
            .map(|ds| ds.game_system.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        systems.sort();
        systems
    }
}
