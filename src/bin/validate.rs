//! Bulk datasheet validator.
//!
//! Walks the `data/` directory, attempts to deserialise every `*.json` file as
//! a [`UnitDatasheet`], and reports all errors in one pass.
//!
//! Usage (run from the project root):
//!   cargo run --bin validate

use administratio_militaris::datasheet::UnitDatasheet;
use std::path::Path;
use walkdir::WalkDir;

fn main() {
    let data_dir = Path::new("tabularium");

    if !data_dir.exists() {
        eprintln!("ERROR: tabularium/ directory not found. Run from the project root.");
        std::process::exit(2);
    }

    let mut errors: Vec<(String, String)> = Vec::new();
    let mut ok: Vec<String> = Vec::new();

    for entry in WalkDir::new(data_dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let display = path.display().to_string();

        match std::fs::read_to_string(path) {
            Err(e) => errors.push((display, format!("could not read file: {e}"))),
            Ok(raw) => match serde_json::from_str::<UnitDatasheet>(&raw) {
                Ok(ds) => ok.push(format!("{display}  ({})", ds.id)),
                Err(e) => errors.push((display, e.to_string())),
            },
        }
    }

    let total = ok.len() + errors.len();

    if errors.is_empty() {
        println!("OK  {}/{} datasheets valid\n", ok.len(), total);
        for line in &ok {
            println!("  [ok]  {line}");
        }
    } else {
        println!(
            "FAIL  {}/{} valid,  {} error(s)\n",
            ok.len(),
            total,
            errors.len()
        );
        for line in &ok {
            println!("  [ok]   {line}");
        }
        for (path, msg) in &errors {
            println!("  [ERR]  {path}");
            println!("         {msg}");
        }
        std::process::exit(1);
    }
}
