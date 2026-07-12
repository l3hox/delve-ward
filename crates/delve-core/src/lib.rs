#![forbid(unsafe_code)]

//! Pure game logic for DelveWard: level model, grid, game state, signals,
//! combat, quests, save data. Must never depend on Bevy or any rendering
//! or windowing crate.

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    fn asset_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets")
            .join(relative)
    }

    #[test]
    fn shipped_dungeon_parses() {
        let raw = std::fs::read_to_string(asset_path("levels/architects_tomb.json")).unwrap();
        let dungeon: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(dungeon["name"], "The Architect's Tomb");
        assert!(!dungeon["levels"].as_array().unwrap().is_empty());
        assert!(dungeon["playerStart"].is_object());
    }
}
