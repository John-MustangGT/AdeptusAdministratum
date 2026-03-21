//! Rust data model mirroring the unit-datasheet JSON schema.
//!
//! All types derive `serde::Deserialize` so a datasheet JSON file can be loaded
//! with a single `serde_json::from_str` call.  `serde_json::Value` is used for
//! free-form fields (stats, constraint params) so the schema stays flexible
//! across game systems.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Top-level datasheet
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UnitDatasheet {
    pub id: String,
    pub name: String,
    pub game_system: String,
    #[serde(default)]
    pub version: Option<String>,
    pub faction: Faction,
    pub battlefield_role: String,
    pub points: Points,
    pub unit_size: UnitSize,
    pub profiles: Vec<ModelProfile>,
    #[serde(default)]
    pub weapons: Weapons,
    #[serde(default)]
    pub default_loadout: Vec<LoadoutEntry>,
    #[serde(default)]
    pub wargear_options: Vec<WargearOption>,
    #[serde(default)]
    pub abilities: Vec<Ability>,
    pub keywords: Keywords,
    #[serde(default)]
    pub leader: Option<Leader>,
    #[serde(default)]
    pub transport: Option<Transport>,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    #[serde(default)]
    pub detachment_rules: Vec<DetachmentRule>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sources: Vec<Source>,
}

// ---------------------------------------------------------------------------
// Faction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Faction {
    pub primary: String,
    #[serde(default)]
    pub sub_factions: Vec<String>,
    #[serde(default)]
    pub allies: Vec<String>,
}

// ---------------------------------------------------------------------------
// Points
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Points {
    pub base: u32,
    #[serde(default)]
    pub per_additional_model: Option<u32>,
    #[serde(default = "default_true")]
    pub free_wargear_included: bool,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Unit size
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UnitSize {
    pub min: u32,
    pub max: u32,
    #[serde(default = "default_step")]
    pub step: u32,
    #[serde(default)]
    pub fixed_sizes: Vec<u32>,
}

fn default_step() -> u32 {
    1
}

impl UnitSize {
    /// Returns `true` if the given model count is a valid size for this unit.
    pub fn is_valid_count(&self, count: u32) -> bool {
        if !self.fixed_sizes.is_empty() {
            return self.fixed_sizes.contains(&count);
        }
        if count < self.min || count > self.max {
            return false;
        }
        let above_min = count - self.min;
        above_min % self.step == 0
    }
}

// ---------------------------------------------------------------------------
// Model profiles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelProfile {
    pub name: String,
    /// Either an exact count or the string "remainder".
    #[serde(default)]
    pub count: Option<ProfileCount>,
    /// Free-form stat block.  Keys and value types vary by game system.
    pub stats: HashMap<String, Value>,
    #[serde(default)]
    pub stat_schema: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ProfileCount {
    Fixed(u32),
    Remainder(String), // "remainder"
}

// ---------------------------------------------------------------------------
// Weapons
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Weapons {
    #[serde(default)]
    pub ranged: Vec<Weapon>,
    #[serde(default)]
    pub melee: Vec<Weapon>,
}

impl Weapons {
    /// Look up a weapon by ID across both ranged and melee lists.
    pub fn find(&self, id: &str) -> Option<&Weapon> {
        self.ranged
            .iter()
            .chain(self.melee.iter())
            .find(|w| w.id == id)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Weapon {
    pub id: String,
    pub name: String,
    /// Flat stat block (empty when `profiles` is used instead).
    pub stats: HashMap<String, Value>,
    #[serde(default)]
    pub abilities: Vec<String>,
    #[serde(default = "default_qty")]
    pub quantity_per_model: u32,
    /// Sub-profiles for multi-mode weapons (e.g. plasma gun standard/supercharge).
    #[serde(default)]
    pub profiles: Vec<WeaponProfile>,
}

fn default_qty() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WeaponProfile {
    pub name: String,
    pub stats: HashMap<String, Value>,
    #[serde(default)]
    pub abilities: Vec<String>,
}

// ---------------------------------------------------------------------------
// Default loadout
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoadoutEntry {
    pub weapon_id: String,
    pub quantity: u32,
    #[serde(default)]
    pub applies_to: Option<LoadoutTarget>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoadoutTarget {
    #[serde(default)]
    pub model_count: Option<u32>,
    #[serde(default)]
    pub profile_name: Option<String>,
}

// ---------------------------------------------------------------------------
// Wargear options
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WargearOption {
    pub id: String,
    pub description: String,
    #[serde(rename = "type")]
    pub option_type: WargearOptionType,
    #[serde(default)]
    pub replaces: Vec<String>,
    #[serde(default)]
    pub adds: Vec<String>,
    #[serde(default)]
    pub points_cost: i32,
    #[serde(default = "default_cost_per")]
    pub cost_per: CostPer,
    #[serde(default)]
    pub limits: WargearLimits,
    #[serde(default)]
    pub mutually_exclusive_with: Vec<String>,
    #[serde(default)]
    pub requires: Option<WargearRequires>,
    #[serde(default)]
    pub is_unique: bool,
    #[serde(default)]
    pub grants_keyword: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WargearOptionType {
    Replace,
    Add,
    Remove,
    Upgrade,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CostPer {
    Model,
    Selection,
}

fn default_cost_per() -> CostPer {
    CostPer::Selection
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct WargearLimits {
    #[serde(default)]
    pub max_per_unit: Option<u32>,
    #[serde(default)]
    pub max_per_army: Option<u32>,
    #[serde(default)]
    pub one_per_n_models: Option<u32>,
    #[serde(default)]
    pub requires_min_unit_size: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WargearRequires {
    #[serde(default)]
    pub option_ids: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub min_unit_size: Option<u32>,
}

// ---------------------------------------------------------------------------
// Abilities
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ability {
    pub name: String,
    pub description: String,
    #[serde(rename = "type", default)]
    pub ability_type: Option<AbilityType>,
    #[serde(default)]
    pub timing: Option<String>,
    #[serde(default)]
    pub range: Option<String>,
    #[serde(default)]
    pub affects: Vec<String>,
    #[serde(default)]
    pub condition: Option<Condition>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AbilityType {
    Core,
    Faction,
    Unit,
    Aura,
    Leader,
    Psychic,
    Prayer,
    Stratagem,
    Detachment,
    Wargear,
}

// ---------------------------------------------------------------------------
// Keywords
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Keywords {
    #[serde(default)]
    pub faction: Vec<String>,
    pub unit: Vec<String>,
    #[serde(default)]
    pub transport_keywords: Vec<String>,
}

// ---------------------------------------------------------------------------
// Leader / Transport
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Leader {
    #[serde(default)]
    pub can_lead: Vec<String>,
    #[serde(default)]
    pub leader_abilities: Vec<Ability>,
    #[serde(default)]
    pub bodyguard_for: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Transport {
    pub capacity: u32,
    #[serde(default)]
    pub restrictions: Vec<TransportRestriction>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TransportRestriction {
    pub rule: TransportRule,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub count_multiplier: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportRule {
    MustHaveKeyword,
    MustNotHaveKeyword,
    CountsAs,
    MaxModelsWithKeyword,
}

// ---------------------------------------------------------------------------
// Constraints & Conditions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Constraint {
    pub id: String,
    pub description: String,
    #[serde(rename = "type")]
    pub constraint_type: ConstraintType,
    #[serde(default = "default_severity")]
    pub severity: Severity,
    #[serde(default)]
    pub params: HashMap<String, Value>,
    #[serde(default)]
    pub condition: Option<Condition>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintType {
    UnitSize,
    WargearLimit,
    ArmyUnique,
    Conditional,
    RequiresKeyword,
    ExcludesKeyword,
    ModelCountPerWargear,
    MaxDuplicatesInArmy,
    ForceOrgSlot,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

fn default_severity() -> Severity {
    Severity::Error
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Condition {
    #[serde(rename = "type")]
    pub condition_type: ConditionType,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub option_id: Option<String>,
    #[serde(default)]
    pub unit_id: Option<String>,
    #[serde(default)]
    pub ability_name: Option<String>,
    #[serde(default)]
    pub value: Option<i64>,
    #[serde(default)]
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConditionType {
    HasKeyword,
    HasWargearOption,
    UnitSizeGte,
    UnitSizeLte,
    HasAbility,
    ArmyIncludesUnit,
    AllOf,
    AnyOf,
    NoneOf,
}

// ---------------------------------------------------------------------------
// Detachment rules / Sources
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DetachmentRule {
    pub detachment: String,
    pub abilities: Vec<Ability>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Source {
    pub name: String,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub url: Option<String>,
}
