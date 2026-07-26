//! The pixel-art HUD, ported from the TS hud canvas: a 640x360 software
//! canvas redrawn every frame and stretched over the window as a UI image.
//! Phase 2 scope: health bar, XP bar, mini inventory panel, HUD messages.
//! Text uses the shared pixel font where the TS drew with browser monospace
//! fonts, so messages render uppercase.

use crate::assets_dir;
use crate::attribute_panel::{AttributePanelState, draw_attribute_panel};
use crate::char_creation::{CharCreation, draw_char_creation};
use crate::dialog_overlay::{DialogOverlayState, QuestManagerRes, draw_dialog_overlay};
use crate::equip_layout::EQUIP_SLOTS;
use crate::ground_items::{GroundItemRender, ItemDb};
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::inventory_overlay::InventoryOverlayState;
use crate::item_tooltip::draw_item_tooltip;
use crate::mouse::MouseState;
use crate::npcs::NpcDb;
use crate::overlay::ActiveOverlay;
use crate::pixel_canvas::{PixelCanvas, Rgba, RgbaImage};
use crate::player::Player;
use crate::quest_log_overlay::draw_quest_log_overlay;
use crate::save_load_overlay::{SaveLoadOverlay, draw_save_load_overlay};
use crate::save_store::FileSaveStore;
use crate::session::{self, Session};
use crate::sign_overlay::{SignOverlayState, draw_sign_overlay};
use crate::stats_panel::draw_stats_panel;
use crate::trading_overlay::{TradingOverlayState, draw_trading_overlay};
use crate::transition::Transition;
use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::widget::NodeImageMode;
use delve_core::combat::PLAYER_ATTACK_COOLDOWN;
use delve_core::entities::EquipSlot;
use delve_core::game_state::{GameState, LayerState, door_key};
use delve_core::grid::Facing;
use delve_core::items::{ItemDatabase, ItemSubtype, ItemType};
use delve_core::save_system::get_all_slot_metadata;
use delve_core::status_effects::{StatusEffect, StatusEffectType, has_effect};
use std::collections::HashMap;

pub const HUD_WIDTH: usize = 640;
pub const HUD_HEIGHT: usize = 360;

/// Stored pixels per HUD drawing unit. TS's HUD canvas is exactly
/// `HUD_WIDTH` x `HUD_HEIGHT` and gets stretched over the viewport with
/// `image-rendering: pixelated`; this port keeps the same drawing grid but
/// stores each unit as a `HUD_SCALE` x `HUD_SCALE` block, so every panel,
/// bar, and glyph lands on exactly the same screen pixels as before while
/// item sprites get enough room to appear at their own resolution (see
/// `PixelCanvas::blit_icon`).
///
/// 2 is the smallest value that fits a 32x32 item sprite in a 24-unit slot's
/// 20-unit inner box (20 * 2 = 40 stored pixels). Raising it costs fill work
/// and memory quadratically and buys nothing for the shipped 32x32 art;
/// setting it to 1 restores TS's exact storage resolution.
pub const HUD_SCALE: usize = 2;
const MARGIN: i32 = 8;

/// Window-pixel cursor position to HUD-canvas (`HUD_WIDTH`x`HUD_HEIGHT`)
/// coordinates. The HUD image renders as an `ImageNode` with
/// `NodeImageMode::Stretch` filling the whole window (`setup_hud`), so this
/// is a straight per-axis scale — no aspect-ratio preservation, no
/// letterbox offset, matching TS's `_screenToHud` (`hudCanvas.ts`).
pub fn screen_to_hud(cursor: Vec2, window: &Window) -> Vec2 {
    Vec2::new(
        cursor.x / window.width() * HUD_WIDTH as f32,
        cursor.y / window.height() * HUD_HEIGHT as f32,
    )
}

// Health bar — bottom-left.
const HEALTH_BAR: (i32, i32, i32, i32) = (MARGIN, HUD_HEIGHT as i32 - MARGIN - 24, 140, 24);
// XP bar — bottom, after the (future) torch and hunger bars.
const XP_BAR: (i32, i32, i32, i32) = (
    MARGIN + 140 + MARGIN + 100 + MARGIN + 80 + MARGIN,
    HUD_HEIGHT as i32 - MARGIN - 24,
    120,
    24,
);
// Inventory panel — bottom-right.
const INVENTORY: (i32, i32, i32, i32) = (
    HUD_WIDTH as i32 - MARGIN - 144,
    HUD_HEIGHT as i32 - MARGIN - 176,
    144,
    176,
);
// Compass — top-left.
const COMPASS: (i32, i32, i32, i32) = (MARGIN, MARGIN, 48, 48);
// Minimap — top-right.
const MINIMAP: (i32, i32, i32, i32) = (HUD_WIDTH as i32 - MARGIN - 128, MARGIN, 128, 128);
const MINIMAP_CELL_SIZE: i32 = 6;
// Torch indicator — right of the health bar.
const TORCH_BAR: (i32, i32, i32, i32) =
    (HEALTH_BAR.0 + HEALTH_BAR.2 + MARGIN, HEALTH_BAR.1, 100, 24);
// Hunger bar — right of the torch indicator.
const HUNGER_BAR: (i32, i32, i32, i32) = (TORCH_BAR.0 + TORCH_BAR.2 + MARGIN, TORCH_BAR.1, 80, 24);
// Status effect icons — above the health bar.
const STATUS_ICONS_X: i32 = HEALTH_BAR.0;
const STATUS_ICONS_Y: i32 = HEALTH_BAR.1 - 20;
const STATUS_ICON_SIZE: i32 = 14;
const STATUS_ICON_GAP: i32 = 4;

const SLOT_SIZE: i32 = 24;
const SLOT_GAP: i32 = 4;
const LOW_HP_THRESHOLD: f64 = 0.25;
const LOW_FUEL_THRESHOLD: f64 = 0.2;
const MESSAGE_DURATION: f32 = 2.5;

// Sword swing overlay, ported from `rendering/swordSwing.ts`.
const SWORD_SWING_DURATION: f32 = 0.25;
const SWORD_SWING_START_ANGLE: f32 = 0.6;
const SWORD_SWING_END_ANGLE: f32 = -1.2;
const SWORD_SWING_SCALE: f32 = 4.0;
const SWORD_SPRITE_SIZE: usize = 32;

// Level-up toast, ported from `hud/levelUpNotification.ts`. Distinct from
// `draw_level_up_hint`'s persistent "press L" prompt below.
const LEVEL_UP_DISPLAY_DURATION: f32 = 3.0;
const LEVEL_UP_FADE_START: f32 = 2.0;

const PANEL_BG: Rgba = Rgba::translucent(10, 8, 12, 0.75);
const PANEL_BORDER: Rgba = Rgba::opaque(0x2a, 0x22, 0x30);
const HP_FILL: Rgba = Rgba::opaque(0xcc, 0x33, 0x33);
const HP_LOW: Rgba = Rgba::opaque(0xff, 0x44, 0x44);
const HP_BG: Rgba = Rgba::opaque(0x1a, 0x0a, 0x0a);
const ACCENT_GOLD: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const TEXT_PRIMARY: Rgba = Rgba::opaque(0xcc, 0xcc, 0xcc);
const TEXT_DIM: Rgba = Rgba::opaque(0x66, 0x66, 0x66);
const SLOT_BG: Rgba = Rgba::opaque(0x1a, 0x16, 0x20);
const SLOT_BORDER: Rgba = Rgba::opaque(0x3a, 0x30, 0x40);
const GOLD_COIN: Rgba = Rgba::opaque(0xda, 0xa5, 0x20);
const XP_FILL: Rgba = Rgba::opaque(0x4a, 0x9e, 0xff);
const XP_BG: Rgba = Rgba::opaque(0x22, 0x22, 0x22);
const MESSAGE_COLOR: (u8, u8, u8) = (0xff, 0x66, 0x44);
const COMPASS_INACTIVE: Rgba = Rgba::opaque(0x44, 0x44, 0x44);
const MINIMAP_BG: Rgba = Rgba::translucent(10, 8, 12, 0.8);
const MINIMAP_WALL: Rgba = Rgba::opaque(0x5a, 0x50, 0x60);
const MINIMAP_FLOOR: Rgba = Rgba::opaque(0x2a, 0x25, 0x30);
const MINIMAP_DOOR: Rgba = Rgba::opaque(0x88, 0x66, 0x44);
const MINIMAP_STAIRS: Rgba = Rgba::opaque(0x44, 0xaa, 0xcc);
const MINIMAP_ENEMY: Rgba = Rgba::opaque(0xcc, 0x33, 0x33);
const MINIMAP_BOULDER: Rgba = Rgba::opaque(0x7a, 0x4a, 0x26);
const TORCH_BG: Rgba = Rgba::opaque(0x1a, 0x12, 0x00);
const TORCH_FILL: Rgba = Rgba::opaque(0xcc, 0x88, 0x33);
const TORCH_LOW: Rgba = Rgba::opaque(0xff, 0x66, 0x00);
const LOW_HUNGER_THRESHOLD: f64 = 0.2;
const HUNGER_BG: Rgba = Rgba::opaque(0x1a, 0x14, 0x08);
const HUNGER_FILL: Rgba = Rgba::opaque(0x8a, 0x9a, 0x5a);
const HUNGER_LOW: Rgba = Rgba::opaque(0xcc, 0x44, 0x00);
const DAMAGE_FLASH_RGB: (u8, u8, u8) = (180, 0, 0);
const STARVATION_TINT: Rgba = Rgba::translucent(100, 60, 0, 0.06);

// Mini-panel mouse-interaction highlights, ported from `inventoryPanel.ts`.
const HOVER_FILL: Rgba = Rgba::translucent(0xe8, 0xc8, 0x4a, 0.3);
const HOVER_BORDER: Rgba = Rgba::opaque(0xe8, 0xc8, 0x4a);
const VALID_DROP_FILL: Rgba = Rgba::translucent(0x44, 0xc8, 0x44, 0.25);
const VALID_DROP_BORDER: Rgba = Rgba::opaque(0x44, 0xcc, 0x44);
const BACKPACK_DROP_FILL: Rgba = Rgba::translucent(0x44, 0xc8, 0x44, 0.15);
const BACKPACK_DROP_BORDER: Rgba = Rgba::translucent(0x44, 0xcc, 0x44, 0.4);

// Status effect screen tints (full-screen overlays).
const BURNING_TINT_RGB: (u8, u8, u8) = (255, 100, 0);
const POISON_TINT_RGB: (u8, u8, u8) = (0, 180, 0);
const SLOW_TINT_RGB: (u8, u8, u8) = (80, 120, 255);
const SLOW_TINT_ALPHA: f32 = 0.06;

// Status effect icons — pixel-art droplet/snowflake/flame, 3-tone each.
const POISON_ICON_BASE: (u8, u8, u8) = (0x22, 0xaa, 0x22);
const POISON_ICON_HIGHLIGHT: (u8, u8, u8) = (0x66, 0xff, 0x66);
const SLOW_ICON_BASE: (u8, u8, u8) = (0x55, 0x88, 0xff);
const SLOW_ICON_CENTER: (u8, u8, u8) = (0xaa, 0xcc, 0xff);
const BURNING_ICON_OUTER: (u8, u8, u8) = (0xff, 0x88, 0x44);
const BURNING_ICON_INNER: (u8, u8, u8) = (0xff, 0xcc, 0x44);
const BURNING_ICON_CORE: (u8, u8, u8) = (0xff, 0xee, 0xaa);

/// Compass letter, matching facing, and the direction it sits from center.
const COMPASS_DIRECTIONS: [(&str, Facing, i32, i32); 4] = [
    ("N", Facing::N, 0, -1),
    ("E", Facing::E, 1, 0),
    ("S", Facing::S, 0, 1),
    ("W", Facing::W, -1, 0),
];

fn equip_slot_color(slot: EquipSlot) -> Rgba {
    match slot {
        EquipSlot::Weapon => Rgba::opaque(0xc0, 0xc0, 0xc0),
        EquipSlot::Head => Rgba::opaque(0x8b, 0x69, 0x14),
        EquipSlot::Chest | EquipSlot::Shield => Rgba::opaque(0x46, 0x82, 0xb4),
        EquipSlot::Legs => Rgba::opaque(0x5c, 0x7a, 0x5c),
        EquipSlot::Hands => Rgba::opaque(0x7a, 0x5c, 0x5c),
        EquipSlot::Feet => Rgba::opaque(0x5c, 0x5c, 0x7a),
        EquipSlot::Ring1 | EquipSlot::Ring2 => Rgba::opaque(0xda, 0xa5, 0x20),
        EquipSlot::Amulet => Rgba::opaque(0x9b, 0x59, 0xb6),
    }
}

pub(crate) fn paperdoll_path(slot: EquipSlot) -> &'static str {
    match slot {
        EquipSlot::Weapon => "sprites/paper/right_hand.png",
        EquipSlot::Shield => "sprites/paper/left_hand.png",
        EquipSlot::Head => "sprites/paper/head.png",
        EquipSlot::Chest => "sprites/paper/torso.png",
        EquipSlot::Hands => "sprites/paper/hands.png",
        EquipSlot::Legs => "sprites/paper/legs.png",
        EquipSlot::Feet => "sprites/paper/feet.png",
        EquipSlot::Ring1 | EquipSlot::Ring2 => "sprites/paper/ring.png",
        EquipSlot::Amulet => "sprites/paper/amulet.png",
    }
}

fn consumable_color(subtype: ItemSubtype) -> Rgba {
    match subtype {
        ItemSubtype::HealthPotion => Rgba::opaque(0xcc, 0x33, 0x33),
        ItemSubtype::TorchOil => Rgba::opaque(0xcc, 0x99, 0x00),
        _ => Rgba::opaque(0x88, 0x88, 0x88),
    }
}

/// JS-like number display: integers without decimals, fractions with one.
pub(crate) fn format_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// CPU-side decoded sprites by asset-relative path; `None` caches a failed
/// load so it isn't retried every frame.
#[derive(Default)]
pub struct IconCache {
    images: HashMap<String, Option<RgbaImage>>,
}

impl IconCache {
    pub(crate) fn get(&mut self, path: &str) -> Option<&RgbaImage> {
        self.images
            .entry(path.to_string())
            .or_insert_with(|| {
                let full_path = assets_dir().join(path);
                match image::open(&full_path) {
                    Ok(decoded) => {
                        let rgba = decoded.to_rgba8();
                        let (width, height) = rgba.dimensions();
                        Some(RgbaImage {
                            width: width as usize,
                            height: height as usize,
                            pixels: rgba.into_raw(),
                        })
                    }
                    Err(error) => {
                        warn!("failed to load HUD sprite {}: {error}", full_path.display());
                        None
                    }
                }
            })
            .as_ref()
    }
}

#[derive(Resource)]
pub struct HudState {
    time: f32,
    message: String,
    message_timer: f32,
    image: Handle<Image>,
    icons: IconCache,
    sword_swing_timer: f32,
    sword_sprite: RgbaImage,
    level_up_message: String,
    level_up_timer: f32,
}

impl HudState {
    /// Show a temporary text message centered on screen.
    pub fn show_message(&mut self, text: &str) {
        self.message = text.to_uppercase();
        self.message_timer = MESSAGE_DURATION;
    }

    /// Start the sword swing overlay, ported from `SwordSwingAnimator.trigger`.
    pub fn trigger_sword_swing(&mut self) {
        self.sword_swing_timer = SWORD_SWING_DURATION;
    }

    /// Start the "LEVEL UP! N" toast, ported from `LevelUpNotification.trigger`.
    pub fn trigger_level_up(&mut self, level: i64) {
        self.level_up_message = format!("LEVEL {level}");
        self.level_up_timer = LEVEL_UP_DISPLAY_DURATION;
    }

    /// Mutable access to the shared icon cache for overlay renderers outside
    /// this module (`inventory_overlay.rs`, `trading_overlay.rs`) that need
    /// [`draw_item_icon`] — `icons` itself stays private so callers can only
    /// reach it through the cache's own load-and-remember behavior.
    pub(crate) fn icons_mut(&mut self) -> &mut IconCache {
        &mut self.icons
    }
}

/// Pixel-art sword: blade, edge highlight, tip, guard, grip, pommel — a 1:1
/// port of the TS `drawSword`'s `fillRect` calls onto a 32x32 canvas.
fn generate_sword_sprite() -> RgbaImage {
    let mut canvas = PixelCanvas::new(SWORD_SPRITE_SIZE);
    canvas.fill_rect(12, 2, 6, 20, Rgba::opaque(0xc0, 0xc8, 0xd0)); // blade
    canvas.fill_rect(14, 2, 2, 20, Rgba::opaque(0xe0, 0xe8, 0xf0)); // edge highlight
    canvas.fill_rect(14, 0, 2, 2, Rgba::opaque(0xd0, 0xd8, 0xe0)); // tip
    canvas.fill_rect(8, 22, 14, 3, Rgba::opaque(0xaa, 0x88, 0x33)); // guard
    canvas.fill_rect(13, 25, 4, 6, Rgba::opaque(0x5a, 0x3a, 0x1a)); // grip
    canvas.fill_rect(13, 31, 4, 1, Rgba::opaque(0xaa, 0x88, 0x33)); // pommel
    let width = canvas.width();
    let height = canvas.height();
    RgbaImage {
        width,
        height,
        pixels: canvas.into_rgba_bytes(),
    }
}

/// `t * (2 - t)`, matching TS's `easeOutQuad`.
fn ease_out_quad(t: f32) -> f32 {
    t * (2.0 - t)
}

/// Sweeps the sword sprite from lower-right to upper-left, ported from
/// `SwordSwingAnimator.draw`. Decrements `hud.sword_swing_timer` itself
/// (like `draw_message` does for its own timer) rather than needing a
/// separate ungated update system.
fn draw_sword_swing(canvas: &mut PixelCanvas, hud: &mut HudState, delta: f32) {
    if hud.sword_swing_timer <= 0.0 {
        return;
    }
    hud.sword_swing_timer = (hud.sword_swing_timer - delta).max(0.0);
    if hud.sword_swing_timer <= 0.0 {
        return;
    }
    let t = 1.0 - hud.sword_swing_timer / SWORD_SWING_DURATION;
    let angle = SWORD_SWING_START_ANGLE
        + (SWORD_SWING_END_ANGLE - SWORD_SWING_START_ANGLE) * ease_out_quad(t);
    let pivot = (HUD_WIDTH as f32 * 0.65, HUD_HEIGHT as f32 * 0.95);
    let draw_edge = SWORD_SPRITE_SIZE as f32 * SWORD_SWING_SCALE;
    let alpha = if t < 0.7 { 1.0 } else { 1.0 - (t - 0.7) / 0.3 };
    canvas.blit_rotated(
        &hud.sword_sprite,
        pivot,
        angle,
        (-draw_edge / 2.0, -draw_edge),
        (draw_edge, draw_edge),
        alpha,
    );
}

/// "LEVEL UP" + the level number, faded in from `LEVEL_UP_FADE_START`
/// remaining seconds to zero — ported from `LevelUpNotification.draw`.
/// Drawn last (see `draw_hud`) so it appears on top of the rest of the HUD,
/// matching TS's own comment on the equivalent call site.
fn draw_level_up_toast(canvas: &mut PixelCanvas, hud: &mut HudState, delta: f32) {
    if hud.level_up_timer <= 0.0 {
        return;
    }
    hud.level_up_timer = (hud.level_up_timer - delta).max(0.0);
    if hud.level_up_timer <= 0.0 {
        return;
    }
    let alpha = if hud.level_up_timer <= LEVEL_UP_FADE_START {
        hud.level_up_timer / LEVEL_UP_FADE_START
    } else {
        1.0
    };
    let scale = 3;
    let line_spacing = 12;
    let label = "LEVEL UP";
    let label_y = 52;
    let label_x = (HUD_WIDTH as i32 - measure_pixel_text(label, scale)) / 2;
    let level_y = label_y + 5 * scale + line_spacing;
    let level_x = (HUD_WIDTH as i32 - measure_pixel_text(&hud.level_up_message, scale)) / 2;
    let color = Rgba::translucent(0xe8, 0xc8, 0x4a, alpha);
    draw_pixel_text(canvas, label, label_x, label_y, color, scale);
    draw_pixel_text(
        canvas,
        &hud.level_up_message,
        level_x,
        level_y,
        color,
        scale,
    );
}

pub fn setup_hud(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut image = Image::new(
        Extent3d {
            width: (HUD_WIDTH * HUD_SCALE) as u32,
            height: (HUD_HEIGHT * HUD_SCALE) as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; HUD_WIDTH * HUD_SCALE * HUD_HEIGHT * HUD_SCALE * 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    let handle = images.add(image);
    commands.spawn((
        ImageNode::new(handle.clone()).with_mode(NodeImageMode::Stretch),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        GlobalZIndex(50),
    ));
    commands.insert_resource(HudState {
        time: 0.0,
        message: String::new(),
        message_timer: 0.0,
        image: handle,
        icons: IconCache::default(),
        sword_swing_timer: 0.0,
        sword_sprite: generate_sword_sprite(),
        level_up_message: String::new(),
        level_up_timer: 0.0,
    });
}

/// Read-only game state the HUD draws from, bundled to stay under the
/// argument-count lint.
#[derive(bevy::ecs::system::SystemParam)]
pub struct HudSources<'w, 's> {
    session: Res<'w, Session>,
    items: Res<'w, ItemDb>,
    creation: Res<'w, CharCreation>,
    vitals: Res<'w, crate::status_effects::PlayerVitals>,
    players: Query<'w, 's, &'static Player>,
    save_load: Res<'w, SaveLoadOverlay>,
    save_store: Res<'w, FileSaveStore>,
    overlay: Res<'w, ActiveOverlay>,
    dialog_state: Res<'w, DialogOverlayState>,
    sign_state: Res<'w, SignOverlayState>,
    quests: Res<'w, QuestManagerRes>,
    mini_panel: Res<'w, MiniPanelState>,
    inventory_state: Res<'w, InventoryOverlayState>,
    attribute_state: Res<'w, AttributePanelState>,
    trading_state: Res<'w, TradingOverlayState>,
    npc_db: Res<'w, NpcDb>,
    mouse: Res<'w, MouseState>,
}

pub fn draw_hud(
    time: Res<Time>,
    mut hud: ResMut<HudState>,
    mut images: ResMut<Assets<Image>>,
    sources: HudSources,
) {
    let hud = &mut *hud;
    let delta = time.delta_secs();
    hud.time += delta;
    let mut canvas = PixelCanvas::supersampled(HUD_WIDTH, HUD_HEIGHT, HUD_SCALE);

    if *sources.overlay == ActiveOverlay::CharCreation {
        draw_char_creation(&mut canvas, &sources.creation);
    } else {
        let game = &sources.session.game;

        // Player damage flash — red overlay, drawn under everything else.
        let flash_alpha = sources.vitals.damage_flash_alpha();
        if flash_alpha > 0.0 {
            let (red, green, blue) = DAMAGE_FLASH_RGB;
            canvas.fill_rect(
                0,
                0,
                HUD_WIDTH as i32,
                HUD_HEIGHT as i32,
                Rgba::translucent(red, green, blue, flash_alpha * 0.4),
            );
        }
        draw_status_screen_tints(&mut canvas, &game.status_fx.player_status_effects, hud.time);
        if game.status_fx.hunger <= 0.0 {
            canvas.fill_rect(0, 0, HUD_WIDTH as i32, HUD_HEIGHT as i32, STARVATION_TINT);
        }

        draw_sword_swing(&mut canvas, hud, delta);

        draw_health_bar(&mut canvas, game.player.hp, game.player.max_hp, hud.time);
        draw_status_icons(&mut canvas, &game.status_fx.player_status_effects, hud.time);
        draw_inventory_panel(
            &mut canvas,
            game,
            &sources.items.0,
            &mut hud.icons,
            &sources.mini_panel,
        );
        draw_xp_bar(&mut canvas, game);
        draw_level_up_hint(&mut canvas, game);
        draw_message(&mut canvas, hud, delta);

        if let Ok(player) = sources.players.single() {
            let player_state = player.grid_state();
            draw_compass(&mut canvas, player_state.facing);
            draw_minimap(
                &mut canvas,
                &sources.session.grid,
                game.active_layer(),
                player_state.col,
                player_state.row,
                player_state.facing,
            );
        }
        draw_torch_indicator(
            &mut canvas,
            game.status_fx.torch_fuel,
            game.status_fx.max_torch_fuel,
            hud.time,
        );
        draw_hunger_bar(
            &mut canvas,
            game.status_fx.hunger,
            game.status_fx.max_hunger,
            hud.time,
        );

        draw_level_up_toast(&mut canvas, hud, delta);
    }

    // Drawn on top of, not instead of, whichever screen rendered above —
    // matches TS's DOM layering, where the overlay sits above the dimmed
    // (but still-rendered) game/HUD rather than replacing it.
    if *sources.overlay == ActiveOverlay::SaveLoad {
        let metadata = get_all_slot_metadata(&*sources.save_store);
        draw_save_load_overlay(&mut canvas, &sources.save_load, &metadata);
    }
    if *sources.overlay == ActiveOverlay::Dialog {
        draw_dialog_overlay(
            &mut canvas,
            &sources.dialog_state,
            &sources.session.game,
            &sources.quests.0,
        );
    }
    if *sources.overlay == ActiveOverlay::Inventory {
        crate::inventory_overlay::draw_inventory_overlay(
            &mut canvas,
            &sources.inventory_state,
            &sources.session.game,
            &sources.items.0,
            hud.icons_mut(),
        );
    }
    if *sources.overlay == ActiveOverlay::Sign {
        draw_sign_overlay(&mut canvas, &sources.sign_state);
    }
    if *sources.overlay == ActiveOverlay::AttributePanel {
        draw_attribute_panel(&mut canvas, &sources.attribute_state, &sources.session.game);
    }
    if *sources.overlay == ActiveOverlay::StatsPanel {
        draw_stats_panel(&mut canvas, &sources.session.game);
    }
    if *sources.overlay == ActiveOverlay::Trading {
        draw_trading_overlay(
            &mut canvas,
            &sources.trading_state,
            &sources.npc_db.0,
            &sources.session.game,
            &sources.items.0,
            &sources.mouse,
            hud.icons_mut(),
        );
    }
    if *sources.overlay == ActiveOverlay::QuestLog {
        draw_quest_log_overlay(&mut canvas, &sources.quests.0);
    }

    if let Some(mut image) = images.get_mut(&hud.image) {
        image.data = Some(canvas.into_rgba_bytes());
    }
}

fn draw_hunger_bar(canvas: &mut PixelCanvas, hunger: f64, max_hunger: f64, time: f32) {
    let (x, y, width, height) = HUNGER_BAR;
    let ratio = (hunger / max_hunger).clamp(0.0, 1.0);

    canvas.fill_rect(x, y, width, height, PANEL_BG);

    let bar_x = x + 20;
    let bar_y = y + 4;
    let bar_w = width - 24;
    let bar_h = height - 8;
    canvas.fill_rect(bar_x, bar_y, bar_w, bar_h, HUNGER_BG);

    let mut fill_color = HUNGER_FILL;
    if ratio <= LOW_HUNGER_THRESHOLD {
        // Slow pulse when low.
        if (time * 6.0).sin() > 0.0 {
            fill_color = HUNGER_LOW;
        }
    }
    canvas.fill_rect(
        bar_x,
        bar_y,
        (f64::from(bar_w) * ratio) as i32,
        bar_h,
        fill_color,
    );

    draw_bread(canvas, x + 5, y + 4, HUNGER_FILL);

    let percent = (ratio * 100.0).round() as i64;
    draw_pixel_text(
        canvas,
        &percent.to_string(),
        bar_x + 2,
        bar_y + 2,
        TEXT_PRIMARY,
        1,
    );

    canvas.stroke_rect(x, y, width, height, PANEL_BORDER);
}

/// 5x7 pixel bread loaf.
fn draw_bread(canvas: &mut PixelCanvas, x: i32, y: i32, color: Rgba) {
    canvas.fill_rect(x + 1, y, 3, 1, color);
    canvas.fill_rect(x, y + 1, 5, 2, color);
    canvas.fill_rect(x, y + 3, 1, 1, color);
    canvas.fill_rect(x + 2, y + 3, 1, 1, color);
    canvas.fill_rect(x + 4, y + 3, 1, 1, color);
    canvas.fill_rect(x, y + 4, 5, 1, color);
    canvas.fill_rect(x + 1, y + 5, 3, 1, color);
    canvas.fill_rect(x + 2, y + 6, 1, 1, color);
}

fn draw_compass(canvas: &mut PixelCanvas, facing: Facing) {
    let (x, y, width, height) = COMPASS;
    let center_x = x + width / 2;
    let center_y = y + height / 2;

    canvas.fill_rect(x, y, width, height, PANEL_BG);
    canvas.stroke_rect(x, y, width, height, PANEL_BORDER);

    let scale = 2;
    let letter_offset = 14;
    for (label, direction, dx, dy) in COMPASS_DIRECTIONS {
        let color = if direction == facing {
            ACCENT_GOLD
        } else {
            COMPASS_INACTIVE
        };
        // Centers the 3px-wide, 5px-tall glyph on the offset point.
        let letter_x = center_x + dx * letter_offset - 3;
        let letter_y = center_y + dy * letter_offset - 5;
        draw_pixel_text(canvas, label, letter_x, letter_y, color, scale);
    }

    canvas.fill_rect(center_x - 1, center_y - 1, 3, 3, ACCENT_GOLD);
}

/// Minimap pixel origin and the grid cell shown at that origin, shared by
/// every dot drawn on top of the terrain grid.
struct MinimapView {
    origin_x: i32,
    origin_y: i32,
    start_col: i32,
    start_row: i32,
}

/// Draws a 3x3 dot centered on `(entity_col, entity_row)`, skipping entities
/// outside explored cells.
fn draw_minimap_dot(
    canvas: &mut PixelCanvas,
    layer: &LayerState,
    view: &MinimapView,
    entity_col: i64,
    entity_row: i64,
    color: Rgba,
) {
    if !layer
        .explored_cells
        .contains(&door_key(entity_col, entity_row))
    {
        return;
    }
    let dot_x = view.origin_x
        + (entity_col as i32 - view.start_col) * MINIMAP_CELL_SIZE
        + MINIMAP_CELL_SIZE / 2;
    let dot_y = view.origin_y
        + (entity_row as i32 - view.start_row) * MINIMAP_CELL_SIZE
        + MINIMAP_CELL_SIZE / 2;
    canvas.fill_rect(dot_x - 1, dot_y - 1, 3, 3, color);
}

fn draw_minimap(
    canvas: &mut PixelCanvas,
    grid: &[String],
    layer: &LayerState,
    player_col: i32,
    player_row: i32,
    facing: Facing,
) {
    let (x, y, width, height) = MINIMAP;

    canvas.fill_rect(x, y, width, height, MINIMAP_BG);

    let visible_cols = width / MINIMAP_CELL_SIZE;
    let visible_rows = height / MINIMAP_CELL_SIZE;
    let start_col = player_col - visible_cols / 2;
    let start_row = player_row - visible_rows / 2;
    let grid_rows = grid.len() as i32;
    let grid_cols = grid.first().map_or(0, |row| row.chars().count() as i32);

    for view_row in 0..visible_rows {
        for view_col in 0..visible_cols {
            let cell_col = start_col + view_col;
            let cell_row = start_row + view_row;
            let key = door_key(i64::from(cell_col), i64::from(cell_row));

            if !layer.explored_cells.contains(&key) {
                continue;
            }
            if cell_row < 0 || cell_row >= grid_rows || cell_col < 0 || cell_col >= grid_cols {
                continue;
            }

            // Persistent (illusory) secret walls still appear as walls on the minimap.
            let cell = grid[cell_row as usize].chars().nth(cell_col as usize);
            let is_wall = cell == Some('#')
                || layer
                    .secret_walls
                    .get(&key)
                    .is_some_and(|wall| wall.persistent);

            let color = if is_wall {
                MINIMAP_WALL
            } else if layer.doors.contains_key(&key) {
                MINIMAP_DOOR
            } else if layer.stairs.contains_key(&key) {
                MINIMAP_STAIRS
            } else {
                MINIMAP_FLOOR
            };

            canvas.fill_rect(
                x + view_col * MINIMAP_CELL_SIZE,
                y + view_row * MINIMAP_CELL_SIZE,
                MINIMAP_CELL_SIZE,
                MINIMAP_CELL_SIZE,
                color,
            );
        }
    }

    let view = MinimapView {
        origin_x: x,
        origin_y: y,
        start_col,
        start_row,
    };
    for enemy in layer.enemies.values() {
        draw_minimap_dot(canvas, layer, &view, enemy.col, enemy.row, MINIMAP_ENEMY);
    }
    for boulder in layer.boulders.values() {
        draw_minimap_dot(
            canvas,
            layer,
            &view,
            boulder.col,
            boulder.row,
            MINIMAP_BOULDER,
        );
    }

    let player_x = x + (player_col - start_col) * MINIMAP_CELL_SIZE + MINIMAP_CELL_SIZE / 2;
    let player_y = y + (player_row - start_row) * MINIMAP_CELL_SIZE + MINIMAP_CELL_SIZE / 2;
    canvas.fill_rect(player_x - 1, player_y - 1, 3, 3, ACCENT_GOLD);

    let (delta_col, delta_row) = facing.delta();
    canvas.stroke_line(
        player_x,
        player_y,
        player_x + delta_col * MINIMAP_CELL_SIZE,
        player_y + delta_row * MINIMAP_CELL_SIZE,
        ACCENT_GOLD,
    );

    canvas.stroke_rect(x, y, width, height, PANEL_BORDER);
}

/// 5x7 pixel flame icon.
fn draw_flame(canvas: &mut PixelCanvas, x: i32, y: i32, color: Rgba) {
    canvas.fill_rect(x + 2, y, 1, 1, color);
    canvas.fill_rect(x + 1, y + 1, 3, 1, color);
    canvas.fill_rect(x + 1, y + 2, 3, 1, color);
    canvas.fill_rect(x, y + 3, 1, 1, color);
    canvas.fill_rect(x + 2, y + 3, 1, 1, color);
    canvas.fill_rect(x + 4, y + 3, 1, 1, color);
    canvas.fill_rect(x, y + 4, 5, 1, color);
    canvas.fill_rect(x + 1, y + 5, 3, 1, color);
    canvas.fill_rect(x + 2, y + 6, 1, 1, color);
}

fn draw_torch_indicator(canvas: &mut PixelCanvas, fuel: f64, max_fuel: f64, time: f32) {
    let (x, y, width, height) = TORCH_BAR;
    let ratio = (fuel / max_fuel).clamp(0.0, 1.0);

    canvas.fill_rect(x, y, width, height, PANEL_BG);

    let bar_x = x + 20;
    let bar_y = y + 4;
    let bar_w = width - 24;
    let bar_h = height - 8;
    canvas.fill_rect(bar_x, bar_y, bar_w, bar_h, TORCH_BG);

    let mut fill_color = TORCH_FILL;
    if ratio <= LOW_FUEL_THRESHOLD {
        let flicker = (time * 10.0).sin() * (time * 7.0).sin();
        fill_color = if flicker > 0.0 { TORCH_LOW } else { TORCH_FILL };
    }
    canvas.fill_rect(
        bar_x,
        bar_y,
        (f64::from(bar_w) * ratio) as i32,
        bar_h,
        fill_color,
    );

    draw_flame(canvas, x + 5, y + 4, TORCH_FILL);

    let percent = (ratio * 100.0).round() as i32;
    draw_pixel_text(
        canvas,
        &percent.to_string(),
        bar_x + 2,
        bar_y + 2,
        TEXT_PRIMARY,
        1,
    );

    canvas.stroke_rect(x, y, width, height, PANEL_BORDER);
}

fn draw_health_bar(canvas: &mut PixelCanvas, hp: f64, max_hp: f64, time: f32) {
    let (x, y, width, height) = HEALTH_BAR;
    let ratio = (hp / max_hp).clamp(0.0, 1.0);

    canvas.fill_rect(x, y, width, height, PANEL_BG);

    let bar_x = x + 20;
    let bar_y = y + 4;
    let bar_w = width - 24;
    let bar_h = height - 8;
    canvas.fill_rect(bar_x, bar_y, bar_w, bar_h, HP_BG);

    let mut fill_color = HP_FILL;
    if ratio <= LOW_HP_THRESHOLD {
        let pulse = (time * 6.0).sin() * 0.5 + 0.5;
        fill_color = if pulse > 0.5 { HP_LOW } else { HP_FILL };
    }
    canvas.fill_rect(
        bar_x,
        bar_y,
        (f64::from(bar_w) * ratio) as i32,
        bar_h,
        fill_color,
    );

    draw_heart(canvas, x + 4, y + 6, HP_FILL);

    let text = format!("{}/{}", format_number(hp), format_number(max_hp));
    draw_pixel_text(canvas, &text, bar_x + 2, bar_y + 2, TEXT_PRIMARY, 1);

    canvas.stroke_rect(x, y, width, height, PANEL_BORDER);
}

/// Simple 7x6 pixel heart.
fn draw_heart(canvas: &mut PixelCanvas, x: i32, y: i32, color: Rgba) {
    canvas.fill_rect(x + 1, y, 2, 1, color);
    canvas.fill_rect(x + 4, y, 2, 1, color);
    canvas.fill_rect(x, y + 1, 7, 2, color);
    canvas.fill_rect(x + 1, y + 3, 5, 1, color);
    canvas.fill_rect(x + 2, y + 4, 3, 1, color);
    canvas.fill_rect(x + 3, y + 5, 1, 1, color);
}

fn draw_key_icon(canvas: &mut PixelCanvas, x: i32, y: i32, color: Rgba) {
    canvas.fill_rect(x + 1, y, 3, 1, color);
    canvas.fill_rect(x, y + 1, 1, 3, color);
    canvas.fill_rect(x + 4, y + 1, 1, 3, color);
    canvas.fill_rect(x + 1, y + 4, 3, 1, color);
    canvas.fill_rect(x + 5, y + 2, 5, 1, color);
    canvas.fill_rect(x + 8, y + 3, 1, 2, color);
    canvas.fill_rect(x + 10, y + 3, 1, 2, color);
}

fn draw_xp_bar(canvas: &mut PixelCanvas, game: &GameState) {
    let (x, y, width, height) = XP_BAR;
    let level = game.player.level;

    canvas.fill_rect(x, y, width, height, PANEL_BG);
    canvas.stroke_rect(x, y, width, height, PANEL_BORDER);

    draw_pixel_text(canvas, &format!("LV{level}"), x + 6, y + 5, ACCENT_GOLD, 2);

    if level >= delve_core::inventory_state::LEVEL_CAP {
        draw_pixel_text(canvas, "MAX", x + 38, y + 6, ACCENT_GOLD, 2);
        return;
    }

    let xp_floor = game.xp_for_level(level - 1);
    let xp_next = game.xp_for_level(level);
    let xp_into_level = game.player.xp - xp_floor;
    let xp_needed = xp_next - xp_floor;
    let ratio = (xp_into_level as f64 / xp_needed as f64).min(1.0);

    let bar_x = x + 36;
    let bar_w = width - 42;
    let bar_y = y + 4;
    canvas.fill_rect(bar_x, bar_y, bar_w, 6, XP_BG);
    canvas.fill_rect(bar_x, bar_y, (f64::from(bar_w) * ratio) as i32, 6, XP_FILL);

    draw_pixel_text(
        canvas,
        &format!("{xp_into_level}/{xp_needed}"),
        bar_x,
        y + 14,
        TEXT_DIM,
        1,
    );
}

fn draw_level_up_hint(canvas: &mut PixelCanvas, game: &GameState) {
    if game.player.attribute_points <= 0 {
        return;
    }
    let hint = "PRESS 'L' TO LEVEL UP";
    let hint_w = measure_pixel_text(hint, 2);
    let (x, y, width, _) = XP_BAR;
    draw_pixel_text(
        canvas,
        hint,
        x + (width - hint_w) / 2,
        y - 12,
        ACCENT_GOLD,
        2,
    );
}

fn draw_slot(canvas: &mut PixelCanvas, x: i32, y: i32) {
    canvas.fill_rect(x, y, SLOT_SIZE, SLOT_SIZE, SLOT_BG);
    canvas.stroke_rect(x, y, SLOT_SIZE, SLOT_SIZE, SLOT_BORDER);
}

/// How an icon's sprite is fitted into its slot.
///
/// TS resamples in both of its modes: its HUD leaves `imageSmoothingEnabled`
/// at the default `true` for the inventory panel and the full inventory
/// overlay, and sets it to `false` in the trading overlay
/// (`tradingOverlay.ts:389`), the one item surface that opts out. Each call
/// site names its own mode so that split stays visible.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconSampling {
    /// Every source pixel preserved at whole-number magnification, centred in
    /// the slot — sharper than either TS mode, made possible by [`HUD_SCALE`].
    /// Sprites too large to fit whole are area-averaged over the slot's stored
    /// pixels instead, still finer than the drawing grid allows.
    Native,
    /// Point-sampled, as `imageSmoothingEnabled = false` does.
    Nearest,
}

/// Draws `item_id`'s real sprite (cached, loaded from `sprites/items/`) into
/// a `size`x`size` square at `(x, y)`, falling back to a colored square plus
/// the item's first initial when the item has no def or its sprite fails to
/// load. `size` is a caller-chosen slot edge length — the mini panel, the
/// full inventory overlay, and the trading overlay each have their own slot
/// size, so this takes it explicitly rather than assuming `hud.rs`'s own
/// `SLOT_SIZE`. Shared across overlay modules via [`HudState::icons_mut`] so
/// every item-bearing surface renders the same real-PNG-or-fallback icon,
/// not a second, drifting copy of this logic.
pub(crate) fn draw_item_icon(
    canvas: &mut PixelCanvas,
    icons: &mut IconCache,
    items: &ItemDatabase,
    item_id: &str,
    slot: (i32, i32, i32),
    fallback_color: Rgba,
    sampling: IconSampling,
) {
    let (x, y, size) = slot;
    let def = items.get_item(item_id);
    let sprite = def
        .map(|def| format!("sprites/items/{}.png", def.icon))
        .and_then(|path| icons.get(&path).map(|_| path));
    if let Some(path) = sprite {
        if let Some(image) = icons.get(&path) {
            let padding = 2;
            let icon_size = size - padding * 2;
            let target = (x + padding, y + padding, icon_size, icon_size);
            match sampling {
                IconSampling::Native => {
                    canvas.blit_icon(image, target, 1.0);
                }
                IconSampling::Nearest => canvas.blit_scaled(image, target, 1.0),
            }
        }
        return;
    }
    canvas.fill_rect(x + 4, y + 4, size - 8, size - 8, fallback_color);
    let label: String = def
        .map_or_else(|| item_id.to_string(), |def| def.name.clone())
        .chars()
        .take(1)
        .collect::<String>()
        .to_uppercase();
    draw_pixel_text(
        canvas,
        &label,
        x + size / 3,
        y + size / 3 - 1,
        Rgba::opaque(0, 0, 0),
        2,
    );
}

// ---------------------------------------------------------------------------
// Mini panel mouse interactions — hover, drag-to-equip, double-click use,
// right-click drop — ported from `hud/inventoryPanel.ts`'s module-level
// `hoveredSlot`/`dragState` and its mouse handlers.
// ---------------------------------------------------------------------------

use crate::inventory_overlay::{
    CursorPos, Section, handle_drop, handle_enter, resolve_drag, valid_equip_slots_for_drag,
};

struct MiniPanelDrag {
    source: CursorPos,
    item_id: String,
    hud_x: f32,
    hud_y: f32,
    valid_equip_slots: std::collections::HashSet<usize>,
}

/// Whether the panel currently blocks mouse-driven dungeon actions is not
/// tracked separately here — `mini_panel_input` gates on `ActiveOverlay`
/// itself. Gated on `overlay.is_open()` (any overlay), not just
/// `Inventory` specifically as TS's own `if (inventoryOverlay.isOpen())
/// return;` checks: TS's window-level mouse listeners stay live underneath
/// the attribute/stats/dialog panels too (the HUD canvas's `pointer-
/// events:none` is only toggled by the inventory overlay), which would let
/// mini-panel clicks land on the inventory through another panel's opaque
/// backdrop — almost certainly an unnoticed gap rather than intended
/// behavior, and not one worth reproducing.
#[derive(Resource, Default)]
pub struct MiniPanelState {
    hovered: Option<CursorPos>,
    drag: Option<MiniPanelDrag>,
}

fn mini_panel_slot_origin(pos: CursorPos) -> (i32, i32) {
    let (x, y, _, _) = INVENTORY;
    let equip_y1 = y + 28;
    let equip_y2 = equip_y1 + SLOT_SIZE + SLOT_GAP;
    let backpack_y = equip_y2 + SLOT_SIZE + SLOT_GAP + 4;
    match pos.section {
        Section::Equipment => {
            let row_y = if pos.index < 5 { equip_y1 } else { equip_y2 };
            let col = pos.index as i32 % 5;
            (x + 6 + col * (SLOT_SIZE + SLOT_GAP), row_y)
        }
        Section::Backpack => {
            let col = pos.index as i32 % 4;
            let row = pos.index as i32 / 4;
            (
                x + 6 + col * (SLOT_SIZE + SLOT_GAP),
                backpack_y + row * (SLOT_SIZE + SLOT_GAP),
            )
        }
    }
}

fn mini_panel_in_slot(hud_x: f32, hud_y: f32, sx: i32, sy: i32) -> bool {
    hud_x >= sx as f32
        && hud_x < (sx + SLOT_SIZE) as f32
        && hud_y >= sy as f32
        && hud_y < (sy + SLOT_SIZE) as f32
}

/// Ported from TS's `panelHitTest` — equipment row 1 (0-4), row 2 (5-9),
/// then the backpack grid (0-11).
fn mini_panel_hit_test(hud_x: f32, hud_y: f32) -> Option<CursorPos> {
    for index in 0..EQUIP_SLOTS.len() {
        let pos = CursorPos {
            section: Section::Equipment,
            index,
        };
        let (sx, sy) = mini_panel_slot_origin(pos);
        if mini_panel_in_slot(hud_x, hud_y, sx, sy) {
            return Some(pos);
        }
    }
    for index in 0..12 {
        let pos = CursorPos {
            section: Section::Backpack,
            index,
        };
        let (sx, sy) = mini_panel_slot_origin(pos);
        if mini_panel_in_slot(hud_x, hud_y, sx, sy) {
            return Some(pos);
        }
    }
    None
}

/// Dependencies `mini_panel_input` needs beyond the mouse/overlay flag,
/// bundled to stay under the argument-count lint.
#[derive(SystemParam)]
pub struct MiniPanelEffects<'w, 's> {
    mini: ResMut<'w, MiniPanelState>,
    session: ResMut<'w, Session>,
    items: Res<'w, ItemDb>,
    players: Query<'w, 's, &'static Player>,
    item_render: GroundItemRender<'w, 's>,
    hud: ResMut<'w, HudState>,
}

/// Ported from `hudCanvas.ts`'s window-level mousemove/mousedown/mouseup/
/// dblclick listeners for the mini panel — see [`MiniPanelState`]'s doc
/// comment for the one deliberate gating deviation from TS. Also blocked
/// during a level transition, matching every other dungeon-context input
/// system's `InputGate::blocked()` shape (TS's own mini-panel listeners
/// have no transition check at all — a second instance of the same
/// window-level-listener gap the module doc comment already flags, not
/// worth reproducing twice).
pub fn mini_panel_input(
    mouse: Res<MouseState>,
    overlay: Res<ActiveOverlay>,
    transition: Res<Transition>,
    mut effects: MiniPanelEffects,
) {
    if transition.is_active() || overlay.is_open() || !mouse.in_window {
        effects.mini.hovered = None;
        return;
    }

    let mut action = None;
    {
        let game = &effects.session.game;
        let items = &effects.items.0;

        if effects.mini.drag.is_some() {
            if let Some(drag) = effects.mini.drag.as_mut() {
                drag.hud_x = mouse.hud_x;
                drag.hud_y = mouse.hud_y;
            }
            effects.mini.hovered = None;
        } else {
            effects.mini.hovered = mini_panel_hit_test(mouse.hud_x, mouse.hud_y);
        }

        if mouse.left_just_pressed
            && effects.mini.drag.is_none()
            && let Some(pos) = mini_panel_hit_test(mouse.hud_x, mouse.hud_y)
        {
            let entity = match pos.section {
                Section::Equipment => game.entity_registry.get_equipped(EQUIP_SLOTS[pos.index]),
                Section::Backpack => game.entity_registry.backpack_item_at(pos.index as u32),
            };
            if let Some(entity) = entity
                && let Some(def) = items.get_item(&entity.item_id)
            {
                effects.mini.drag = Some(MiniPanelDrag {
                    source: pos,
                    item_id: entity.item_id.clone(),
                    hud_x: mouse.hud_x,
                    hud_y: mouse.hud_y,
                    valid_equip_slots: valid_equip_slots_for_drag(
                        def.item_type,
                        def.subtype,
                        pos.section,
                        game,
                    ),
                });
            }
        }

        if mouse.left_just_released
            && let Some(drag) = effects.mini.drag.take()
            && let Some(target) = mini_panel_hit_test(mouse.hud_x, mouse.hud_y)
        {
            action = resolve_drag(drag.source, target, game, items);
        }

        if action.is_none()
            && mouse.left_double_clicked
            && let Some(pos) = mini_panel_hit_test(mouse.hud_x, mouse.hud_y)
        {
            action = handle_enter(pos, game, items);
        }

        if action.is_none()
            && mouse.right_just_pressed
            && let Some(pos) = mini_panel_hit_test(mouse.hud_x, mouse.hud_y)
            && let Ok(player) = effects.players.single()
        {
            let player_state = player.grid_state();
            action = handle_drop(
                pos,
                game,
                i64::from(player_state.col),
                i64::from(player_state.row),
            );
        }
    }

    if let Some(action) = action {
        session::apply_inventory_action(
            &mut effects.session.game,
            &mut effects.item_render,
            &mut effects.hud,
            &action,
        );
    }
}

fn draw_inventory_panel(
    canvas: &mut PixelCanvas,
    game: &GameState,
    items: &ItemDatabase,
    icons: &mut IconCache,
    mini_panel: &MiniPanelState,
) {
    let (x, y, width, height) = INVENTORY;

    canvas.fill_rect(x, y, width, height, PANEL_BG);

    // Key count and gold on the top row.
    let key_count = game.picked_up_keys().len();
    draw_key_icon(canvas, x + 6, y + 6, ACCENT_GOLD);
    draw_pixel_text(
        canvas,
        &format!("x{key_count}"),
        x + 20,
        y + 8,
        TEXT_PRIMARY,
        2,
    );

    canvas.fill_ellipse(
        (x + width - 50) as f32,
        (y + 11) as f32,
        4.0,
        4.0,
        GOLD_COIN,
    );
    draw_pixel_text(
        canvas,
        &format!("{}G", game.player.gold),
        x + width - 42,
        y + 8,
        GOLD_COIN,
        2,
    );

    // Equipment: two rows of five slots.
    let equip_y1 = y + 28;
    let equip_y2 = equip_y1 + SLOT_SIZE + SLOT_GAP;
    for (index, &slot) in EQUIP_SLOTS.iter().enumerate() {
        let slot_x = x + 6 + (index as i32 % 5) * (SLOT_SIZE + SLOT_GAP);
        let slot_y = if index < 5 { equip_y1 } else { equip_y2 };
        let pos = CursorPos {
            section: Section::Equipment,
            index,
        };
        if mini_panel.drag.is_none() && mini_panel.hovered == Some(pos) {
            canvas.fill_rect(
                slot_x - 1,
                slot_y - 1,
                SLOT_SIZE + 2,
                SLOT_SIZE + 2,
                HOVER_FILL,
            );
            canvas.stroke_rect(
                slot_x - 1,
                slot_y - 1,
                SLOT_SIZE + 2,
                SLOT_SIZE + 2,
                HOVER_BORDER,
            );
        }
        if mini_panel.drag.as_ref().is_some_and(|drag| {
            drag.source.section == Section::Backpack && drag.valid_equip_slots.contains(&index)
        }) {
            canvas.fill_rect(
                slot_x - 1,
                slot_y - 1,
                SLOT_SIZE + 2,
                SLOT_SIZE + 2,
                VALID_DROP_FILL,
            );
            canvas.stroke_rect(
                slot_x - 1,
                slot_y - 1,
                SLOT_SIZE + 2,
                SLOT_SIZE + 2,
                VALID_DROP_BORDER,
            );
        }
        draw_slot(canvas, slot_x, slot_y);
        if let Some(entity) = game.entity_registry.get_equipped(slot) {
            let item_id = entity.item_id.clone();
            draw_item_icon(
                canvas,
                icons,
                items,
                &item_id,
                (slot_x, slot_y, SLOT_SIZE),
                equip_slot_color(slot),
                IconSampling::Native,
            );
        } else if let Some(ghost) = icons.get(paperdoll_path(slot)) {
            let pad = 3;
            canvas.blit_icon(
                ghost,
                (
                    slot_x + pad,
                    slot_y + pad,
                    SLOT_SIZE - pad * 2,
                    SLOT_SIZE - pad * 2,
                ),
                0.3,
            );
        }
    }

    // Weapon cooldown overlay on the first slot.
    if game.player.attack_cooldown > 0.0 {
        let cooldown_ratio = game.player.attack_cooldown / PLAYER_ATTACK_COOLDOWN;
        let fill_h = (f64::from(SLOT_SIZE) * cooldown_ratio).ceil() as i32;
        canvas.fill_rect(
            x + 6,
            equip_y1 + (SLOT_SIZE - fill_h),
            SLOT_SIZE,
            fill_h,
            Rgba::translucent(200, 60, 60, 0.45),
        );
    }

    // Backpack: 12 slots, 4 columns x 3 rows.
    let backpack_y = equip_y2 + SLOT_SIZE + SLOT_GAP + 4;
    for row in 0..3 {
        for column in 0..4 {
            let slot_index = row * 4 + column;
            let slot_x = x + 6 + column * (SLOT_SIZE + SLOT_GAP);
            let slot_y = backpack_y + row * (SLOT_SIZE + SLOT_GAP);
            let pos = CursorPos {
                section: Section::Backpack,
                index: slot_index as usize,
            };
            if mini_panel.drag.is_none() && mini_panel.hovered == Some(pos) {
                canvas.fill_rect(
                    slot_x - 1,
                    slot_y - 1,
                    SLOT_SIZE + 2,
                    SLOT_SIZE + 2,
                    HOVER_FILL,
                );
                canvas.stroke_rect(
                    slot_x - 1,
                    slot_y - 1,
                    SLOT_SIZE + 2,
                    SLOT_SIZE + 2,
                    HOVER_BORDER,
                );
            }
            if mini_panel.drag.as_ref().is_some_and(|drag| {
                !(drag.source.section == Section::Backpack
                    && drag.source.index == slot_index as usize)
            }) {
                canvas.fill_rect(
                    slot_x - 1,
                    slot_y - 1,
                    SLOT_SIZE + 2,
                    SLOT_SIZE + 2,
                    BACKPACK_DROP_FILL,
                );
                canvas.stroke_rect(
                    slot_x - 1,
                    slot_y - 1,
                    SLOT_SIZE + 2,
                    SLOT_SIZE + 2,
                    BACKPACK_DROP_BORDER,
                );
            }
            draw_slot(canvas, slot_x, slot_y);

            let entity = game.entity_registry.backpack_item_at(slot_index as u32);
            let def = entity.and_then(|entity| items.get_item(&entity.item_id));
            if let Some(entity) = entity {
                let fallback = def.map_or(Rgba::opaque(0x88, 0x88, 0x88), |def| {
                    if def.item_type == ItemType::Consumable {
                        consumable_color(def.subtype)
                    } else {
                        Rgba::opaque(0x88, 0x88, 0x88)
                    }
                });
                let item_id = entity.item_id.clone();
                draw_item_icon(
                    canvas,
                    icons,
                    items,
                    &item_id,
                    (slot_x, slot_y, SLOT_SIZE),
                    fallback,
                    IconSampling::Native,
                );
            }

            // Quick-use slot numbers 1-8: gold for consumables, grey otherwise.
            if slot_index < 8 {
                let number_x = slot_x + SLOT_SIZE - 8;
                let number_y = slot_y + 1;
                let is_consumable = def.is_some_and(|def| def.item_type == ItemType::Consumable);
                canvas.fill_rect(number_x, number_y, 6, 7, Rgba::translucent(0, 0, 0, 0.7));
                draw_pixel_text(
                    canvas,
                    &(slot_index + 1).to_string(),
                    number_x + 1,
                    number_y,
                    if is_consumable {
                        Rgba::opaque(0xff, 0xcc, 0x44)
                    } else {
                        Rgba::opaque(0x66, 0x66, 0x66)
                    },
                    1,
                );
            }
        }
    }

    canvas.stroke_rect(x, y, width, height, PANEL_BORDER);

    if mini_panel.drag.is_none()
        && let Some(pos) = mini_panel.hovered
    {
        let hovered_entity = match pos.section {
            Section::Equipment => game.entity_registry.get_equipped(EQUIP_SLOTS[pos.index]),
            Section::Backpack => game.entity_registry.backpack_item_at(pos.index as u32),
        };
        if let Some(entity) = hovered_entity {
            draw_item_tooltip(canvas, entity, game, items, x - 4, y);
        }
    }

    if let Some(drag) = &mini_panel.drag {
        draw_item_icon(
            canvas,
            icons,
            items,
            &drag.item_id,
            (
                drag.hud_x as i32 - SLOT_SIZE / 2,
                drag.hud_y as i32 - SLOT_SIZE / 2,
                SLOT_SIZE,
            ),
            Rgba::opaque(0x88, 0x88, 0x88),
            IconSampling::Native,
        );
    }
}

fn draw_message(canvas: &mut PixelCanvas, hud: &mut HudState, delta: f32) {
    if hud.message_timer <= 0.0 {
        return;
    }
    hud.message_timer -= delta;
    let alpha = (hud.message_timer / 0.5).clamp(0.0, 1.0);
    let text_w = measure_pixel_text(&hud.message, 2);
    let x = (HUD_WIDTH as i32 - text_w) / 2;
    let y = HUD_HEIGHT as i32 / 2 - 5;
    draw_pixel_text(
        canvas,
        &hud.message,
        x + 1,
        y + 1,
        Rgba::translucent(0, 0, 0, alpha),
        2,
    );
    let (red, green, blue) = MESSAGE_COLOR;
    draw_pixel_text(
        canvas,
        &hud.message,
        x,
        y,
        Rgba::translucent(red, green, blue, alpha),
        2,
    );
}

fn rgba((red, green, blue): (u8, u8, u8), alpha: f32) -> Rgba {
    Rgba::translucent(red, green, blue, alpha)
}

/// Full-screen color washes for active status effects: burning and poison
/// pulse via a sine wave, slow is a constant tint.
fn draw_status_screen_tints(canvas: &mut PixelCanvas, effects: &[StatusEffect], time: f32) {
    let (width, height) = (HUD_WIDTH as i32, HUD_HEIGHT as i32);
    if has_effect(effects, StatusEffectType::Burning) {
        let alpha = 0.08 + 0.04 * (time * 12.0).sin();
        canvas.fill_rect(0, 0, width, height, rgba(BURNING_TINT_RGB, alpha));
    }
    if has_effect(effects, StatusEffectType::Poison) {
        let alpha = 0.06 + 0.02 * (time * 4.0).sin();
        canvas.fill_rect(0, 0, width, height, rgba(POISON_TINT_RGB, alpha));
    }
    if has_effect(effects, StatusEffectType::Slow) {
        canvas.fill_rect(0, 0, width, height, rgba(SLOW_TINT_RGB, SLOW_TINT_ALPHA));
    }
}

/// Green droplet icon for poison.
fn draw_poison_icon(canvas: &mut PixelCanvas, x: i32, y: i32, size: i32, alpha: f32) {
    let center_x = x + size / 2;
    let pixel = size / 7;
    let base = rgba(POISON_ICON_BASE, alpha);
    canvas.fill_rect(center_x - pixel, y + pixel, pixel * 2, pixel, base); // top narrow
    canvas.fill_rect(center_x - pixel * 2, y + pixel * 2, pixel * 4, pixel, base); // middle
    canvas.fill_rect(
        center_x - pixel * 3,
        y + pixel * 3,
        pixel * 6,
        pixel * 2,
        base,
    ); // wide
    canvas.fill_rect(center_x - pixel * 2, y + pixel * 5, pixel * 4, pixel, base); // bottom narrow
    canvas.fill_rect(
        center_x - pixel,
        y + pixel * 3,
        pixel,
        pixel,
        rgba(POISON_ICON_HIGHLIGHT, alpha),
    );
}

/// Blue snowflake icon for slow.
fn draw_slow_icon(canvas: &mut PixelCanvas, x: i32, y: i32, size: i32, alpha: f32) {
    let center_x = x + size / 2;
    let center_y = y + size / 2;
    let pixel = size / 7;
    let base = rgba(SLOW_ICON_BASE, alpha);
    canvas.fill_rect(
        center_x - pixel,
        center_y - pixel * 3,
        pixel * 2,
        pixel * 6,
        base,
    ); // vertical
    canvas.fill_rect(
        center_x - pixel * 3,
        center_y - pixel,
        pixel * 6,
        pixel * 2,
        base,
    ); // horizontal
    canvas.fill_rect(
        center_x - pixel * 2,
        center_y - pixel * 2,
        pixel,
        pixel,
        base,
    );
    canvas.fill_rect(center_x + pixel, center_y - pixel * 2, pixel, pixel, base);
    canvas.fill_rect(center_x - pixel * 2, center_y + pixel, pixel, pixel, base);
    canvas.fill_rect(center_x + pixel, center_y + pixel, pixel, pixel, base);
    canvas.fill_rect(
        center_x - pixel / 2,
        center_y - pixel / 2,
        pixel,
        pixel,
        rgba(SLOW_ICON_CENTER, alpha),
    );
}

/// Orange flame icon for burning.
fn draw_burning_icon(canvas: &mut PixelCanvas, x: i32, y: i32, size: i32, alpha: f32) {
    let center_x = x + size / 2;
    let pixel = size / 7;
    let outer = rgba(BURNING_ICON_OUTER, alpha);
    canvas.fill_rect(center_x - pixel, y + pixel, pixel * 2, pixel, outer); // tip
    canvas.fill_rect(center_x - pixel * 2, y + pixel * 2, pixel * 4, pixel, outer); // upper
    canvas.fill_rect(
        center_x - pixel * 2,
        y + pixel * 3,
        pixel * 4,
        pixel * 2,
        outer,
    ); // middle
    canvas.fill_rect(center_x - pixel * 3, y + pixel * 5, pixel * 6, pixel, outer); // base
    canvas.fill_rect(
        center_x - pixel,
        y + pixel * 3,
        pixel * 2,
        pixel * 2,
        rgba(BURNING_ICON_INNER, alpha),
    );
    canvas.fill_rect(
        center_x - pixel / 2,
        y + pixel * 4,
        pixel,
        pixel,
        rgba(BURNING_ICON_CORE, alpha),
    );
}

/// Active status effect icons above the health bar, deduplicated by type.
fn draw_status_icons(canvas: &mut PixelCanvas, effects: &[StatusEffect], time: f32) {
    if effects.is_empty() {
        return;
    }
    // Gentle pulse: alpha scales between 0.7 and 1.0.
    let pulse = 0.85 + 0.15 * (time * 3.0).sin();
    let mut shown = Vec::new();
    let mut offset_x = 0;
    for effect in effects {
        if shown.contains(&effect.effect_type) {
            continue;
        }
        shown.push(effect.effect_type);

        let x = STATUS_ICONS_X + offset_x;
        match effect.effect_type {
            StatusEffectType::Poison => {
                draw_poison_icon(canvas, x, STATUS_ICONS_Y, STATUS_ICON_SIZE, pulse)
            }
            StatusEffectType::Slow => {
                draw_slow_icon(canvas, x, STATUS_ICONS_Y, STATUS_ICON_SIZE, pulse)
            }
            StatusEffectType::Burning => {
                draw_burning_icon(canvas, x, STATUS_ICONS_Y, STATUS_ICON_SIZE, pulse);
            }
        }
        offset_x += STATUS_ICON_SIZE + STATUS_ICON_GAP;
    }
}
