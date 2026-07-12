//! The pixel-art HUD, ported from the TS hud canvas: a 640x360 software
//! canvas redrawn every frame and stretched over the window as a UI image.
//! Phase 2 scope: health bar, XP bar, mini inventory panel, HUD messages.
//! Text uses the shared pixel font where the TS drew with browser monospace
//! fonts, so messages render uppercase.

use crate::assets_dir;
use crate::char_creation::{CharCreation, draw_char_creation};
use crate::ground_items::ItemDb;
use crate::hud_font::{draw_pixel_text, measure_pixel_text};
use crate::pixel_canvas::{PixelCanvas, Rgba, RgbaImage};
use crate::session::Session;
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::widget::NodeImageMode;
use delve_core::combat::PLAYER_ATTACK_COOLDOWN;
use delve_core::entities::EquipSlot;
use delve_core::game_state::GameState;
use delve_core::items::{ItemDatabase, ItemSubtype, ItemType};
use std::collections::HashMap;

pub const HUD_WIDTH: usize = 640;
pub const HUD_HEIGHT: usize = 360;
const MARGIN: i32 = 8;

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

const SLOT_SIZE: i32 = 24;
const SLOT_GAP: i32 = 4;
const LOW_HP_THRESHOLD: f64 = 0.25;
const MESSAGE_DURATION: f32 = 2.5;

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

/// Two rows of five equipment slots, matching the TS panel layout.
const EQUIP_SLOTS: [EquipSlot; 10] = [
    EquipSlot::Weapon,
    EquipSlot::Head,
    EquipSlot::Chest,
    EquipSlot::Legs,
    EquipSlot::Hands,
    EquipSlot::Shield,
    EquipSlot::Feet,
    EquipSlot::Ring1,
    EquipSlot::Ring2,
    EquipSlot::Amulet,
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

fn paperdoll_path(slot: EquipSlot) -> &'static str {
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
fn format_number(value: f64) -> String {
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
    fn get(&mut self, path: &str) -> Option<&RgbaImage> {
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
}

impl HudState {
    /// Show a temporary text message centered on screen.
    pub fn show_message(&mut self, text: &str) {
        self.message = text.to_uppercase();
        self.message_timer = MESSAGE_DURATION;
    }
}

pub fn setup_hud(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut image = Image::new(
        Extent3d {
            width: HUD_WIDTH as u32,
            height: HUD_HEIGHT as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; HUD_WIDTH * HUD_HEIGHT * 4],
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
    });
}

pub fn draw_hud(
    time: Res<Time>,
    mut hud: ResMut<HudState>,
    mut images: ResMut<Assets<Image>>,
    session: Res<Session>,
    items: Res<ItemDb>,
    creation: Res<CharCreation>,
) {
    let hud = &mut *hud;
    let delta = time.delta_secs();
    hud.time += delta;
    let mut canvas = PixelCanvas::with_dimensions(HUD_WIDTH, HUD_HEIGHT);

    if creation.active {
        draw_char_creation(&mut canvas, &creation);
    } else {
        let game = &session.game;
        draw_health_bar(&mut canvas, game.player.hp, game.player.max_hp, hud.time);
        draw_inventory_panel(&mut canvas, game, &items.0, &mut hud.icons);
        draw_xp_bar(&mut canvas, game);
        draw_level_up_hint(&mut canvas, game);
        draw_message(&mut canvas, hud, delta);
    }

    if let Some(mut image) = images.get_mut(&hud.image) {
        image.data = Some(canvas.into_rgba_bytes());
    }
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

fn draw_item_icon(
    canvas: &mut PixelCanvas,
    icons: &mut IconCache,
    items: &ItemDatabase,
    item_id: &str,
    slot_x: i32,
    slot_y: i32,
    fallback_color: Rgba,
) {
    let def = items.get_item(item_id);
    let sprite = def
        .map(|def| format!("sprites/items/{}.png", def.icon))
        .and_then(|path| icons.get(&path).map(|_| path));
    if let Some(path) = sprite {
        if let Some(image) = icons.get(&path) {
            let padding = 2;
            let icon_size = SLOT_SIZE - padding * 2;
            canvas.blit_scaled(
                image,
                (slot_x + padding, slot_y + padding, icon_size, icon_size),
                1.0,
            );
        }
        return;
    }
    canvas.fill_rect(
        slot_x + 4,
        slot_y + 4,
        SLOT_SIZE - 8,
        SLOT_SIZE - 8,
        fallback_color,
    );
    let label: String = def
        .map_or_else(|| item_id.to_string(), |def| def.name.clone())
        .chars()
        .take(1)
        .collect::<String>()
        .to_uppercase();
    draw_pixel_text(
        canvas,
        &label,
        slot_x + 8,
        slot_y + 7,
        Rgba::opaque(0, 0, 0),
        2,
    );
}

fn draw_inventory_panel(
    canvas: &mut PixelCanvas,
    game: &GameState,
    items: &ItemDatabase,
    icons: &mut IconCache,
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
        draw_slot(canvas, slot_x, slot_y);
        if let Some(entity) = game.entity_registry.get_equipped(slot) {
            let item_id = entity.item_id.clone();
            draw_item_icon(
                canvas,
                icons,
                items,
                &item_id,
                slot_x,
                slot_y,
                equip_slot_color(slot),
            );
        } else if let Some(ghost) = icons.get(paperdoll_path(slot)) {
            let pad = 3;
            canvas.blit_scaled(
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
                draw_item_icon(canvas, icons, items, &item_id, slot_x, slot_y, fallback);
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
