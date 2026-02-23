use adeptus_administratum::datasheet::UnitDatasheet;
use adeptus_administratum::validation::{calculate_points, validate_unit, UnitSelection};
use std::process;

fn main() {
    // Load the Battle Sisters Squad datasheet bundled at compile time.
    let json = include_str!("../data/40k/adepta-sororitas/battle-sisters-squad.json");

    let datasheet: UnitDatasheet = match serde_json::from_str(json) {
        Ok(ds) => ds,
        Err(e) => {
            eprintln!("Failed to parse datasheet: {e}");
            process::exit(1);
        }
    };

    println!("=== {} ===", datasheet.name);
    println!(
        "Faction: {} | Role: {} | Base pts: {}",
        datasheet.faction.primary, datasheet.battlefield_role, datasheet.points.base
    );
    println!(
        "Unit size: {}-{} (step {})\n",
        datasheet.unit_size.min, datasheet.unit_size.max, datasheet.unit_size.step
    );

    // Example selection: 10-model unit with 2 meltaguns, a power weapon, and Simulacrum Imperialis.
    let selection = UnitSelection {
        model_count: 10,
        chosen_options: vec![
            "special_weapon_melta".into(),
            "special_weapon_melta".into(),
            "sister_superior_power_weapon".into(),
            "simulacrum_imperialis".into(),
        ],
        active_keywords: datasheet
            .keywords
            .unit
            .iter()
            .chain(datasheet.keywords.faction.iter())
            .cloned()
            .collect(),
        ..Default::default()
    };

    println!("--- Selected loadout ---");
    println!("Models: {}", selection.model_count);
    println!("Options: {:?}", selection.chosen_options);

    let pts = calculate_points(&datasheet, &selection);
    println!("Total points: {pts}");

    println!("\n--- Validation ---");
    match validate_unit(&datasheet, &selection) {
        Ok(issues) if issues.is_empty() => {
            println!("OK — no constraint violations.");
        }
        Ok(issues) => {
            for issue in &issues {
                println!("[{:?}] {}: {}", issue.severity, issue.constraint_id, issue.message);
            }
        }
        Err(e) => {
            eprintln!("Validator structural error: {e}");
            process::exit(2);
        }
    }

    // Demonstrate an invalid selection.
    println!("\n--- Invalid selection (3 meltaguns in 10-model unit) ---");
    let bad_selection = UnitSelection {
        model_count: 10,
        chosen_options: vec![
            "special_weapon_melta".into(),
            "special_weapon_melta".into(),
            "special_weapon_melta".into(),
        ],
        active_keywords: selection.active_keywords.clone(),
        ..Default::default()
    };

    match validate_unit(&datasheet, &bad_selection) {
        Ok(issues) if issues.is_empty() => println!("OK (unexpected — should have issues)"),
        Ok(issues) => {
            for issue in &issues {
                println!("[{:?}] {}: {}", issue.severity, issue.constraint_id, issue.message);
            }
        }
        Err(e) => eprintln!("Validator structural error: {e}"),
    }
}
