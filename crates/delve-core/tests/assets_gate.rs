//! Phase 0 gate: every shipped JSON file in `assets/levels/` and `assets/data/`
//! parses into the typed model and passes validation.

use delve_core::dialogs::{DialogTree, dialog_layout_from_json};
use delve_core::enemies::EnemyDatabase;
use delve_core::items::ItemDatabase;
use delve_core::level_loader::{ValidationContext, validate_dungeon_str, validate_level_str};
use delve_core::loot::LootTables;
use delve_core::npcs::NpcDatabase;
use delve_core::quests::QuestDef;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn json_files(directory: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to list {}: {error}", directory.display()))
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no JSON files in {}",
        directory.display()
    );
    files
}

fn databases() -> (EnemyDatabase, NpcDatabase) {
    let enemies = EnemyDatabase::from_json(&read(&assets_dir().join("data/enemies.json")))
        .expect("enemies.json loads");
    let npcs = NpcDatabase::from_json(&read(&assets_dir().join("data/npcs.json")))
        .expect("npcs.json loads");
    (enemies, npcs)
}

#[test]
fn every_shipped_level_file_validates() {
    let (enemies, npcs) = databases();
    let enemy_ids = enemies.all_enemy_ids();
    let npc_ids = npcs.all_npc_ids();
    let ctx = ValidationContext {
        enemy_ids: Some(&enemy_ids),
        npc_ids: Some(&npc_ids),
    };

    let mut failures: Vec<String> = Vec::new();
    for path in json_files(&assets_dir().join("levels")) {
        let source = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8 file name");
        let json = read(&path);
        let is_dungeon = serde_json::from_str::<serde_json::Value>(&json)
            .map(|value| value.get("levels").is_some())
            .unwrap_or(false);

        let mut warnings = Vec::new();
        let result = if is_dungeon {
            validate_dungeon_str(&json, source, &ctx, &mut warnings).map(|_| ())
        } else {
            validate_level_str(&json, source, &ctx, &mut warnings).map(|_| ())
        };
        for warning in &warnings {
            println!("warning: {warning}");
        }
        if let Err(error) = result {
            failures.push(error);
        }
    }
    assert!(
        failures.is_empty(),
        "level validation failures: {failures:#?}"
    );
}

#[test]
fn every_shipped_data_file_parses() {
    let data_dir = assets_dir().join("data");

    ItemDatabase::from_json(&read(&data_dir.join("items.json"))).expect("items.json loads");
    EnemyDatabase::from_json(&read(&data_dir.join("enemies.json"))).expect("enemies.json loads");
    NpcDatabase::from_json(&read(&data_dir.join("npcs.json"))).expect("npcs.json loads");
    LootTables::from_json(&read(&data_dir.join("loot-tables.json")))
        .expect("loot-tables.json loads");

    let known_data_files: HashSet<&str> = [
        "items.json",
        "enemies.json",
        "npcs.json",
        "loot-tables.json",
    ]
    .into();
    for path in json_files(&data_dir) {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8");
        assert!(
            known_data_files.contains(name),
            "unexpected data file {name} — extend the gate test to cover it"
        );
    }

    for path in json_files(&data_dir.join("quests")) {
        QuestDef::from_json(&read(&path))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    }

    for path in json_files(&data_dir.join("dialogs")) {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf-8");
        let json = read(&path);
        if name.ends_with(".layout.json") {
            dialog_layout_from_json(&json)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        } else {
            let tree = DialogTree::from_json(&json)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                tree.nodes.contains_key(&tree.start_node),
                "{name}: startNode {:?} missing from nodes",
                tree.start_node
            );
        }
    }
}

#[test]
fn npc_dialog_references_resolve_to_dialog_files() {
    // nameless_girl has no dialog file in the TS repo either (its dialog would
    // 404 at runtime there too) — tracked in planning/PROGRESS.md Known Issues.
    let known_missing_dialogs: HashSet<&str> = ["nameless_girl"].into();

    let (_, npcs) = databases();
    let dialogs_dir = assets_dir().join("data/dialogs");
    for npc in npcs.all_npcs() {
        if known_missing_dialogs.contains(npc.dialog.as_str()) {
            continue;
        }
        let dialog_path = dialogs_dir.join(format!("{}.json", npc.dialog));
        assert!(
            dialog_path.is_file(),
            "npc {} references missing dialog file {}",
            npc.id,
            dialog_path.display()
        );
    }
}
