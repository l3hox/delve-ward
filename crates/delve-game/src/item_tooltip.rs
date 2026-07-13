//! Item tooltip: quality-colored name, type/subtype, stat lines,
//! comparison deltas against the equipped item in the same slot, stat
//! requirements, and a wrapped description — ported from `hud/itemTooltip.ts`.
//! Drawn by the inventory overlay next to the cursor-selected slot, hidden
//! during a drag.

use crate::hud::HUD_WIDTH;
use crate::hud_font::draw_pixel_text;
use crate::pixel_canvas::{PixelCanvas, Rgba};
use delve_core::entities::{EquipSlot, ItemEntity};
use delve_core::game_state::GameState;
use delve_core::items::{ItemDatabase, ItemDef, ItemQuality, ItemSubtype};

const QUALITY_POOR: Rgba = Rgba::opaque(0x99, 0x99, 0x99);
const QUALITY_COMMON: Rgba = Rgba::opaque(0xcc, 0xcc, 0xcc);
const QUALITY_FINE: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);
const QUALITY_MASTERWORK: Rgba = Rgba::opaque(0x4a, 0x9e, 0xff);
const QUALITY_ENCHANTED: Rgba = Rgba::opaque(0xc8, 0x44, 0xcc);

/// Ported from TS's `getQualityColor`. `ItemQuality` is a closed Rust enum
/// (no `#[serde(other)]` catch-all, unlike `ItemType`/`ItemSubtype`), so
/// TS's "unknown quality falls back to common's color" case has no
/// reachable Rust equivalent — every variant is matched exhaustively.
#[must_use]
pub fn get_quality_color(quality: ItemQuality) -> Rgba {
    match quality {
        ItemQuality::Poor => QUALITY_POOR,
        ItemQuality::Common => QUALITY_COMMON,
        ItemQuality::Fine => QUALITY_FINE,
        ItemQuality::Masterwork => QUALITY_MASTERWORK,
        ItemQuality::Enchanted => QUALITY_ENCHANTED,
    }
}

pub struct StatLine {
    pub label: &'static str,
    pub value: f64,
}

/// `(field accessor, display label)`, in TS's declared display order.
type StatAccessor = fn(&delve_core::items::ItemStats) -> Option<f64>;
const STAT_LABELS: [(StatAccessor, &str); 10] = [
    (|stats| stats.atk, "ATK"),
    (|stats| stats.def, "DEF"),
    (|stats| stats.hp, "HP"),
    (|stats| stats.mp, "MP"),
    (|stats| stats.str, "STR"),
    (|stats| stats.dex, "DEX"),
    (|stats| stats.vit, "VIT"),
    (|stats| stats.wis, "WIS"),
    (|stats| stats.crit_chance, "CRIT%"),
    (|stats| stats.dodge_chance, "DODGE%"),
];

/// Ported from TS's `getStatLines`: only non-zero stats are shown.
#[must_use]
pub fn get_stat_lines(def: &ItemDef) -> Vec<StatLine> {
    STAT_LABELS
        .iter()
        .filter_map(|&(accessor, label)| {
            let value = accessor(&def.stats)?;
            (value != 0.0).then_some(StatLine { label, value })
        })
        .collect()
}

pub struct DeltaLine {
    pub label: &'static str,
    pub delta: f64,
}

/// Ported from TS's `getComparisonDeltas`: per-stat delta of `def` against
/// `equipped`, zero-deltas and a missing `equipped` both producing no line.
#[must_use]
pub fn get_comparison_deltas(def: &ItemDef, equipped: Option<&ItemDef>) -> Vec<DeltaLine> {
    let Some(equipped) = equipped else {
        return Vec::new();
    };
    STAT_LABELS
        .iter()
        .filter_map(|&(accessor, label)| {
            let delta =
                accessor(&def.stats).unwrap_or(0.0) - accessor(&equipped.stats).unwrap_or(0.0);
            (delta != 0.0).then_some(DeltaLine { label, delta })
        })
        .collect()
}

/// Comparison-only subtype→slot mapping: unlike
/// `equip_layout::subtype_to_equip_slot`, rings always compare against
/// ring1 regardless of what's actually equipped — ported from TS's
/// `_subtypeToEquipSlotForComparison` (a deliberately simpler, stateless
/// sibling of the real allocation logic, kept separate in TS too).
fn subtype_to_equip_slot_for_comparison(subtype: ItemSubtype) -> EquipSlot {
    match subtype {
        ItemSubtype::Sword
        | ItemSubtype::Axe
        | ItemSubtype::Dagger
        | ItemSubtype::Mace
        | ItemSubtype::Spear
        | ItemSubtype::Staff => EquipSlot::Weapon,
        ItemSubtype::Head => EquipSlot::Head,
        ItemSubtype::Chest => EquipSlot::Chest,
        ItemSubtype::Legs => EquipSlot::Legs,
        ItemSubtype::Hands => EquipSlot::Hands,
        ItemSubtype::Feet => EquipSlot::Feet,
        ItemSubtype::Shield => EquipSlot::Shield,
        ItemSubtype::Ring => EquipSlot::Ring1,
        ItemSubtype::Amulet => EquipSlot::Amulet,
        _ => EquipSlot::Weapon,
    }
}

const TOOLTIP_WIDTH: i32 = 150;
const TOOLTIP_PADDING: i32 = 6;
const LINE_HEIGHT: i32 = 8;
const TEXT_SCALE: i32 = 1;
const MAX_LINE_CHARS: usize = 25;

const BG: Rgba = Rgba::translucent(10, 8, 12, 0.92);
const BORDER: Rgba = Rgba::opaque(0x2a, 0x22, 0x30);
const TYPE_TEXT: Rgba = Rgba::opaque(0x66, 0x66, 0x66);
const STAT_TEXT: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);
const DELTA_HEADER_TEXT: Rgba = Rgba::opaque(0x55, 0x55, 0x55);
const DELTA_POSITIVE: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);
const DELTA_NEGATIVE: Rgba = Rgba::opaque(0xcc, 0x44, 0x44);
const REQ_MET: Rgba = Rgba::opaque(0x55, 0x55, 0x55);
const REQ_UNMET: Rgba = Rgba::opaque(0xcc, 0x44, 0x44);
const DESC_TEXT: Rgba = Rgba::opaque(0x55, 0x55, 0x55);

struct Line {
    text: String,
    color: Rgba,
}

/// Greedy word-wrap at `max_chars` columns.
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate_len = if current.is_empty() {
            word.len()
        } else {
            current.len() + 1 + word.len()
        };
        if candidate_len > max_chars && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn requirement_lines(def: &ItemDef, game: &GameState) -> Vec<Line> {
    let mut entries: Vec<(&str, f64)> = Vec::new();
    if let Some(value) = def.requirements.str.filter(|value| *value > 0.0) {
        entries.push(("STR", value));
    }
    if let Some(value) = def.requirements.dex.filter(|value| *value > 0.0) {
        entries.push(("DEX", value));
    }
    if let Some(value) = def.requirements.vit.filter(|value| *value > 0.0) {
        entries.push(("VIT", value));
    }
    if let Some(value) = def.requirements.wis.filter(|value| *value > 0.0) {
        entries.push(("WIS", value));
    }
    if entries.is_empty() {
        return Vec::new();
    }
    let effective = game.get_effective_stats();
    entries
        .into_iter()
        .map(|(label, required)| {
            let effective_value = match label {
                "STR" => effective.effective_str,
                "DEX" => effective.effective_dex,
                "VIT" => effective.effective_vit,
                _ => effective.effective_wis,
            };
            let met = effective_value >= required;
            Line {
                text: format!("REQ: {label} {required}"),
                color: if met { REQ_MET } else { REQ_UNMET },
            }
        })
        .collect()
}

/// Draws the tooltip for `entity` anchored near `(x, y)`: right edge only
/// clamps by flipping to the slot's left when it would overflow past
/// `HUD_WIDTH` — TS has no vertical clamp either, so none is added here.
pub fn draw_item_tooltip(
    canvas: &mut PixelCanvas,
    entity: &ItemEntity,
    game: &GameState,
    items: &ItemDatabase,
    x: i32,
    y: i32,
) {
    let Some(def) = items.get_item(&entity.item_id) else {
        return;
    };

    let compare_slot = subtype_to_equip_slot_for_comparison(def.subtype);
    let equipped_entity = game.entity_registry.get_equipped(compare_slot);
    let is_self =
        equipped_entity.is_some_and(|equipped| equipped.instance_id == entity.instance_id);
    let equipped_def = (!is_self)
        .then(|| equipped_entity.and_then(|equipped| items.get_item(&equipped.item_id)))
        .flatten();

    let mut lines = Vec::new();
    lines.push(Line {
        text: def.name.to_uppercase(),
        color: get_quality_color(def.quality),
    });

    let type_label = format!("{:?}", def.item_type).to_uppercase();
    let subtype_label = format!("{:?}", def.subtype).to_uppercase();
    let type_text = if matches!(def.item_type, delve_core::items::ItemType::Consumable) {
        type_label
    } else {
        format!("{type_label} - {subtype_label}")
    };
    lines.push(Line {
        text: type_text,
        color: TYPE_TEXT,
    });

    for stat in get_stat_lines(def) {
        let sign = if stat.value > 0.0 { "+" } else { "" };
        lines.push(Line {
            text: format!("{} {sign}{}", stat.label, stat.value),
            color: STAT_TEXT,
        });
    }

    let can_equip = !matches!(def.item_type, delve_core::items::ItemType::Consumable);
    if can_equip && let Some(equipped_def) = equipped_def {
        let deltas = get_comparison_deltas(def, Some(equipped_def));
        if !deltas.is_empty() {
            lines.push(Line {
                text: "VS EQUIPPED:".to_string(),
                color: DELTA_HEADER_TEXT,
            });
            for delta in deltas {
                let sign = if delta.delta > 0.0 { "+" } else { "" };
                lines.push(Line {
                    text: format!("  {sign}{} {}", delta.delta, delta.label),
                    color: if delta.delta > 0.0 {
                        DELTA_POSITIVE
                    } else {
                        DELTA_NEGATIVE
                    },
                });
            }
        }
    }

    lines.extend(requirement_lines(def, game));

    if !def.description.is_empty() {
        for wrapped in wrap_text(&def.description.to_uppercase(), MAX_LINE_CHARS) {
            lines.push(Line {
                text: wrapped,
                color: DESC_TEXT,
            });
        }
    }

    let box_h = TOOLTIP_PADDING * 2 + lines.len() as i32 * LINE_HEIGHT;
    let adjusted_x = if x + TOOLTIP_WIDTH > HUD_WIDTH as i32 {
        x - TOOLTIP_WIDTH - 4
    } else {
        x
    };

    canvas.fill_rect(adjusted_x, y, TOOLTIP_WIDTH, box_h, BG);
    canvas.stroke_rect(adjusted_x, y, TOOLTIP_WIDTH, box_h, BORDER);

    let mut line_y = y + TOOLTIP_PADDING;
    for line in &lines {
        draw_pixel_text(
            canvas,
            &line.text,
            adjusted_x + TOOLTIP_PADDING,
            line_y,
            line.color,
            TEXT_SCALE,
        );
        line_y += LINE_HEIGHT;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use delve_core::items::{ItemRequirements, ItemStats, ItemType};

    fn item(stats: ItemStats) -> ItemDef {
        ItemDef {
            id: "test_item".to_string(),
            name: "Test Item".to_string(),
            item_type: ItemType::Weapon,
            subtype: ItemSubtype::Sword,
            quality: ItemQuality::Common,
            icon: String::new(),
            weight: 1.0,
            value: 1.0,
            description: String::new(),
            stats,
            modifiers: Vec::new(),
            requirements: ItemRequirements::default(),
            stackable: None,
            stack_max: None,
            effect: None,
        }
    }

    #[test]
    fn quality_colors_cover_every_variant() {
        assert_eq!(get_quality_color(ItemQuality::Poor), QUALITY_POOR);
        assert_eq!(get_quality_color(ItemQuality::Common), QUALITY_COMMON);
        assert_eq!(get_quality_color(ItemQuality::Fine), QUALITY_FINE);
        assert_eq!(
            get_quality_color(ItemQuality::Masterwork),
            QUALITY_MASTERWORK
        );
        assert_eq!(get_quality_color(ItemQuality::Enchanted), QUALITY_ENCHANTED);
    }

    #[test]
    fn stat_lines_skip_zero_and_absent_stats() {
        let def = item(ItemStats {
            atk: Some(5.0),
            crit_chance: Some(2.0),
            def: Some(0.0),
            ..Default::default()
        });
        let lines = get_stat_lines(&def);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].label, "ATK");
        assert!((lines[0].value - 5.0).abs() < 1e-9);
        assert_eq!(lines[1].label, "CRIT%");
    }

    #[test]
    fn comparison_deltas_empty_without_equipped() {
        let def = item(ItemStats {
            atk: Some(5.0),
            ..Default::default()
        });
        assert!(get_comparison_deltas(&def, None).is_empty());
    }

    #[test]
    fn comparison_deltas_positive_when_new_item_is_better() {
        let new_def = item(ItemStats {
            atk: Some(5.0),
            ..Default::default()
        });
        let old_def = item(ItemStats {
            atk: Some(3.0),
            ..Default::default()
        });
        let deltas = get_comparison_deltas(&new_def, Some(&old_def));
        let atk_delta = deltas.iter().find(|delta| delta.label == "ATK").unwrap();
        assert!((atk_delta.delta - 2.0).abs() < 1e-9);
    }

    #[test]
    fn comparison_deltas_negative_when_new_item_is_worse() {
        let new_def = item(ItemStats {
            atk: Some(3.0),
            ..Default::default()
        });
        let old_def = item(ItemStats {
            atk: Some(5.0),
            ..Default::default()
        });
        let deltas = get_comparison_deltas(&new_def, Some(&old_def));
        let atk_delta = deltas.iter().find(|delta| delta.label == "ATK").unwrap();
        assert!((atk_delta.delta - (-2.0)).abs() < 1e-9);
    }

    #[test]
    fn comparison_deltas_skip_zero_delta() {
        let def = item(ItemStats {
            atk: Some(5.0),
            ..Default::default()
        });
        assert!(get_comparison_deltas(&def, Some(&def)).is_empty());
    }

    #[test]
    fn comparison_deltas_treat_missing_stat_as_zero() {
        let def_with_def = item(ItemStats {
            def: Some(8.0),
            ..Default::default()
        });
        let def_without = item(ItemStats {
            atk: Some(3.0),
            ..Default::default()
        });
        let deltas = get_comparison_deltas(&def_with_def, Some(&def_without));
        let def_delta = deltas.iter().find(|delta| delta.label == "DEF").unwrap();
        assert!((def_delta.delta - 8.0).abs() < 1e-9);
    }
}
