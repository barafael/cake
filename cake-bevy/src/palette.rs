//! Player colours, the few fixed colours of the map, and the fills of
//! everything clickable.

use bevy::prelude::*;
use bevy::sprite_render::ColorMaterial;

/// One colour per seat, bright against the dark background and distinct from
/// each other at polyline thickness.
const SEATS: [Color; 8] = [
    Color::srgb(0.95, 0.33, 0.30), // red
    Color::srgb(0.30, 0.60, 1.00), // blue
    Color::srgb(0.40, 0.85, 0.35), // green
    Color::srgb(0.98, 0.82, 0.25), // yellow
    Color::srgb(0.75, 0.45, 0.95), // purple
    Color::srgb(1.00, 0.58, 0.20), // orange
    Color::srgb(0.30, 0.90, 0.88), // cyan
    Color::srgb(0.98, 0.50, 0.75), // pink
];

pub const SEAT_COUNT: usize = SEATS.len();

pub fn seat(i: usize) -> Color {
    SEATS[i % SEATS.len()]
}

/// A seat's colour, faded once that player is out.
pub fn seat_status(i: usize, alive: bool) -> Color {
    seat(i).with_alpha(if alive { 1.0 } else { 0.3 })
}

pub const BACKGROUND: Color = Color::srgb(0.06, 0.065, 0.08);
pub const RING: Color = Color::srgb(0.42, 0.44, 0.50);
pub const FAINT: Color = Color::srgba(0.42, 0.44, 0.50, 0.25);
pub const BLIP: Color = Color::srgb(0.70, 0.72, 0.76);
pub const SELECTED: Color = Color::srgb(0.95, 0.95, 0.95);
pub const GOOD: Color = Color::srgb(0.35, 0.95, 0.45);
pub const BAD: Color = Color::srgb(1.0, 0.30, 0.25);
pub const TEXT: Color = Color::srgb(0.88, 0.89, 0.92);
pub const DIM_TEXT: Color = Color::srgb(0.55, 0.57, 0.62);

/// Fills of clickable things: buttons, menu slots and window slices.
pub const CONTROL_IDLE: Color = Color::srgba(1.0, 1.0, 1.0, 0.07);
pub const CONTROL_HOVER: Color = Color::srgba(1.0, 1.0, 1.0, 0.18);
pub const CONTROL_PRESSED: Color = Color::srgba(1.0, 1.0, 1.0, 0.26);
pub const CONTROL_OFF: Color = Color::srgba(1.0, 1.0, 1.0, 0.03);
/// A control that is switched on, like the pin.
pub const CONTROL_LIT: Color = Color::srgba(1.0, 1.0, 1.0, 0.12);
/// The chosen one of a row of choices.
pub const CONTROL_CHOSEN: Color = Color::srgba(1.0, 1.0, 1.0, 0.24);
pub const CLOSE_HOVER: Color = Color::srgba(0.9, 0.2, 0.2, 0.5);

/// Recolour a material, touching it only when the colour really changes (a
/// change re-prepares it on the GPU).
pub fn tint(materials: &mut Assets<ColorMaterial>, handle: &Handle<ColorMaterial>, color: Color) {
    if let Some(mut m) = materials.get_mut(handle)
        && m.color != color
    {
        m.color = color;
    }
}
