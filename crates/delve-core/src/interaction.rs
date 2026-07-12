//! Player "use" interaction against the facing cell: doors, levers, sconces,
//! blocks, chests, NPCs, fountains, altars, signs, and bookshelves.

use crate::game_state::{ChestState, DoorState, GameState, UsableState};
use crate::grid::{PlayerState, get_facing_cell, is_walkable, walkable_cells};
use crate::status_effect_state::BuffStat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionType {
    DoorOpened,
    DoorClosed,
    DoorBlocked,
    DoorLocked,
    LeverActivated,
    SconceTaken,
    BlockPushed,
    ChestOpened,
    ChestLocked,
    SignRead,
    NpcInteracted,
    FountainUsed,
    BookshelfRead,
    AltarActivated,
    Nothing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InteractionResult {
    pub result_type: InteractionType,
    pub message: Option<String>,
    /// Entity IDs of affected doors (for mesh updates).
    pub targets: Option<Vec<String>>,
    pub target_col: Option<i64>,
    pub target_row: Option<i64>,
}

impl InteractionResult {
    fn of(result_type: InteractionType) -> Self {
        Self {
            result_type,
            message: None,
            targets: None,
            target_col: None,
            target_row: None,
        }
    }

    fn with_message(result_type: InteractionType, message: &str) -> Self {
        Self {
            message: Some(message.to_string()),
            ..Self::of(result_type)
        }
    }
}

fn buff_label(stat: BuffStat) -> &'static str {
    match stat {
        BuffStat::Atk => "ATK",
        BuffStat::Def => "DEF",
        BuffStat::Str => "STR",
        BuffStat::Dex => "DEX",
        BuffStat::Vit => "VIT",
        BuffStat::Wis => "WIS",
    }
}

fn fmt_num(value: f64) -> String {
    if value.fract() == 0.0 && value.is_finite() {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

#[allow(clippy::too_many_lines)]
pub fn interact(
    player_state: &PlayerState,
    grid: &[String],
    game_state: &mut GameState,
) -> InteractionResult {
    let (facing_col, facing_row) = get_facing_cell(player_state);
    let (col, row) = (i64::from(facing_col), i64::from(facing_row));
    let (player_col, player_row) = (i64::from(player_state.col), i64::from(player_state.row));

    let rows = grid.len() as i64;
    let cols = grid.first().map_or(0, |line| line.chars().count()) as i64;
    if row < 0 || row >= rows || col < 0 || col >= cols {
        return InteractionResult::of(InteractionType::Nothing);
    }

    // Door interaction — entity-based lookup (no special grid char).
    if let Some(door) = game_state.get_door(col, row) {
        let (state, mechanical, key_id) = (door.state, door.mechanical, door.key_id.clone());
        if state == DoorState::Open {
            if game_state.is_blocked_by_enemy(col, row) {
                return InteractionResult::with_message(
                    InteractionType::DoorBlocked,
                    "Something is blocking the door.",
                );
            }
            if game_state.close_door(col, row) {
                return InteractionResult::with_message(
                    InteractionType::DoorClosed,
                    "Door closed.",
                );
            }
            return InteractionResult::of(InteractionType::Nothing); // mechanical door
        }

        if state == DoorState::Closed {
            if mechanical {
                return InteractionResult::with_message(
                    InteractionType::Nothing,
                    "This door is operated by a mechanism.",
                );
            }
            if key_id.is_some_and(|key_id| !game_state.has_key(&key_id)) {
                return InteractionResult::with_message(
                    InteractionType::DoorLocked,
                    "This door is locked.",
                );
            }
            if game_state.open_door(col, row) {
                return InteractionResult::with_message(
                    InteractionType::DoorOpened,
                    "Door opened.",
                );
            }
        }
    }

    // Lever — player stands on the lever cell and faces its wall.
    let lever_matches = game_state
        .get_lever(player_col, player_row)
        .is_some_and(|lever| lever.wall == player_state.facing);
    if lever_matches {
        if let Some(targets) = game_state.activate_lever(player_col, player_row) {
            return InteractionResult {
                targets: Some(targets),
                ..InteractionResult::with_message(InteractionType::LeverActivated, "Lever pulled.")
            };
        }
    }

    // Sconce — player stands on the sconce cell and faces its wall.
    let sconce_matches = game_state
        .get_sconce(player_col, player_row)
        .is_some_and(|sconce| sconce.lit && sconce.wall == player_state.facing);
    if sconce_matches && game_state.take_sconce_torch(player_col, player_row) {
        return InteractionResult::with_message(
            InteractionType::SconceTaken,
            "Torch taken. Fuel replenished.",
        );
    }

    // Block push — player faces a cell containing a pushable block.
    if game_state.get_block(col, row).is_some() {
        let (dcol, drow) = player_state.facing.delta();
        let (dest_col, dest_row) = (col + i64::from(dcol), row + i64::from(drow));
        let dest_walkable = {
            let door_open = |c: i32, r: i32| game_state.is_door_open(i64::from(c), i64::from(r));
            i32::try_from(dest_col).is_ok_and(|dc| {
                i32::try_from(dest_row).is_ok_and(|dr| {
                    is_walkable(grid, dc, dr, &walkable_cells(), Some(&door_open), None)
                })
            })
        };
        let can_push = dest_walkable
            && !game_state.is_blocked_by_enemy(dest_col, dest_row)
            && !game_state.is_block_at(dest_col, dest_row)
            && !(dest_col == player_col && dest_row == player_row)
            && !game_state.is_edge_blocked(col, row, dest_col, dest_row);
        if can_push {
            game_state.push_block(col, row, dest_col, dest_row);
            return InteractionResult {
                target_col: Some(dest_col),
                target_row: Some(dest_row),
                ..InteractionResult::of(InteractionType::BlockPushed)
            };
        }
        return InteractionResult::of(InteractionType::Nothing);
    }

    // Chest interaction.
    if let Some(chest) = game_state.get_chest(col, row) {
        if chest.gate_mode.is_some() {
            return InteractionResult::with_message(
                InteractionType::Nothing,
                "This chest is sealed by a mechanism.",
            );
        }
        let result = game_state.open_chest(col, row);
        if result.locked {
            return InteractionResult::with_message(
                InteractionType::ChestLocked,
                "This chest is locked.",
            );
        }
        if result.opened {
            return InteractionResult {
                target_col: Some(col),
                target_row: Some(row),
                ..InteractionResult::with_message(InteractionType::ChestOpened, "Chest opened.")
            };
        }
        return InteractionResult::of(InteractionType::Nothing);
    }

    // NPC interaction — player faces an NPC on the adjacent cell.
    if let Some(npc) = game_state.get_npc(col, row) {
        return InteractionResult {
            target_col: Some(col),
            target_row: Some(row),
            ..InteractionResult::with_message(InteractionType::NpcInteracted, &npc.npc_id)
        };
    }

    // Fountain — facing cell.
    if let Some(fountain) = game_state.get_fountain(col, row) {
        if fountain.state == UsableState::Used {
            return InteractionResult::with_message(
                InteractionType::Nothing,
                "The fountain has dried up.",
            );
        }
        let (healed, heal_amount) = game_state.use_fountain(col, row);
        if healed {
            return InteractionResult {
                target_col: Some(col),
                target_row: Some(row),
                ..InteractionResult::with_message(
                    InteractionType::FountainUsed,
                    &format!("Restored {} HP.", fmt_num(heal_amount)),
                )
            };
        }
    }

    // Altar — facing cell.
    if let Some(altar) = game_state.get_altar(col, row) {
        if altar.state == UsableState::Used {
            return InteractionResult::with_message(
                InteractionType::Nothing,
                "The altar has gone dark.",
            );
        }
        let (activated, buff_type, buff_amount, buff_duration) = game_state.use_altar(col, row);
        if activated {
            return InteractionResult {
                target_col: Some(col),
                target_row: Some(row),
                ..InteractionResult::with_message(
                    InteractionType::AltarActivated,
                    &format!(
                        "{} +{} for {}s",
                        buff_label(buff_type),
                        fmt_num(buff_amount),
                        fmt_num(buff_duration)
                    ),
                )
            };
        }
    }

    // Sign — player stands on the sign cell and faces the sign's wall.
    if let Some(sign) = game_state.get_sign_on_wall(player_col, player_row, player_state.facing) {
        return InteractionResult::with_message(InteractionType::SignRead, &sign.text.clone());
    }

    // Bookshelf — player stands on the cell and faces the bookshelf's wall.
    if let Some(bookshelf) =
        game_state.get_bookshelf_on_wall(player_col, player_row, player_state.facing)
    {
        return InteractionResult::with_message(
            InteractionType::BookshelfRead,
            &bookshelf.text.clone(),
        );
    }

    InteractionResult::of(InteractionType::Nothing)
}
