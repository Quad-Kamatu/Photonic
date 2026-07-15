//! Generate `features.json` — the marketing feature manifest consumed by
//! theunnamed.dev/photonic — from the curated `features/catalog.json`.
//!
//! The catalog owns the editorial copy (name, description, category, GUI/AI
//! availability). This generator layers the *live* keyboard shortcuts on top:
//! for every feature that links a `commandId`, it resolves the shortcut from the
//! app's own command registry (`commands::default_binding`) so the keybinds on
//! the marketing page can never drift from what the app actually ships. Any
//! `commandId` that isn't a real registered command is a hard error, so typos
//! fail the build instead of silently shipping a missing shortcut.
//!
//! Run:
//! ```sh
//! cargo run -p photonic-gui --bin dump_features \
//!   > crates/photonic-gui/features/features.json
//! ```

use std::collections::BTreeSet;

use photonic_gui::commands;
use serde::{Deserialize, Serialize};

/// The curated source of truth, hand-edited and vendored next to this binary.
const CATALOG_JSON: &str = include_str!("../../features/catalog.json");

#[derive(Deserialize)]
struct Catalog {
    categories: Vec<String>,
    features: Vec<CatalogFeature>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogFeature {
    category: String,
    avail: String,
    name: String,
    desc: String,
    #[serde(default)]
    command_id: Option<String>,
}

/// The emitted per-feature record. `keybind`/`keybind_storage`/`command_id` are
/// only present when the feature links a command that carries a default binding.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OutFeature {
    category: String,
    avail: String,
    name: String,
    desc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    keybind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keybind_storage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    /// So the marketing loader (and any human reading the file) knows it's generated.
    generated_by: &'static str,
    note: &'static str,
    categories: Vec<String>,
    features: Vec<OutFeature>,
}

fn main() {
    let catalog: Catalog = serde_json::from_str(CATALOG_JSON)
        .unwrap_or_else(|e| fail(&format!("catalog.json is not valid JSON: {e}")));

    // The universe of real command ids (registry + tool activations), for validation.
    let known: BTreeSet<String> = commands::all_commands()
        .into_iter()
        .map(|c| c.id.to_string())
        .collect();

    let valid_cats: BTreeSet<&str> = catalog.categories.iter().map(String::as_str).collect();

    let mut out = Vec::with_capacity(catalog.features.len());
    let mut bound = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for f in &catalog.features {
        if !valid_cats.contains(f.category.as_str()) {
            errors.push(format!(
                "feature {:?} has category {:?} not in the catalog's category list",
                f.name, f.category
            ));
        }
        if !matches!(f.avail.as_str(), "gui" | "ai" | "both") {
            errors.push(format!(
                "feature {:?} has avail {:?} (expected gui|ai|both)",
                f.name, f.avail
            ));
        }

        let (keybind, keybind_storage) = match &f.command_id {
            Some(id) => {
                if !known.contains(id) {
                    errors.push(format!(
                        "feature {:?} links commandId {:?} which is not a registered command",
                        f.name, id
                    ));
                    (None, None)
                } else if let Some(kb) = commands::default_binding(id) {
                    bound += 1;
                    (Some(kb.display()), Some(kb.to_storage_string()))
                } else {
                    // Known command, but no default shortcut (e.g. a tool). Not an
                    // error — the feature just renders without a keybind chip.
                    (None, None)
                }
            }
            None => (None, None),
        };

        out.push(OutFeature {
            category: f.category.clone(),
            avail: f.avail.clone(),
            name: f.name.clone(),
            desc: f.desc.clone(),
            keybind,
            keybind_storage,
            command_id: f.command_id.clone(),
        });
    }

    if !errors.is_empty() {
        for e in &errors {
            eprintln!("error: {e}");
        }
        fail(&format!("{} catalog problem(s); refusing to emit", errors.len()));
    }

    let manifest = Manifest {
        generated_by: "photonic-gui dump_features",
        note: "GENERATED. Do not hand-edit. Source: crates/photonic-gui/features/catalog.json. Regenerate with `cargo run -p photonic-gui --bin dump_features`.",
        categories: catalog.categories,
        features: out,
    };

    match serde_json::to_string_pretty(&manifest) {
        Ok(s) => println!("{s}"),
        Err(e) => fail(&format!("failed to serialize manifest: {e}")),
    }

    eprintln!(
        "dump_features: {} features across {} categories, {} with keybinds",
        manifest.features.len(),
        manifest.categories.len(),
        bound
    );
}

fn fail(msg: &str) -> ! {
    eprintln!("dump_features: {msg}");
    std::process::exit(1);
}
