//! Text that runs along a circular arc, centred on an angle of the ring.
//!
//! Each character is its own `Text2d`, placed along the arc and turned to
//! follow it. The arc's radius is in circle units. A label's angle is either
//! on the map ([`Frame::Map`]: a name stays beside its sector as the view
//! turns) or on the screen ([`Frame::Screen`]: a menu's title stays at the
//! bottom whichever way the view is turned). Either way the label zooms and
//! pans with the map, like everything inside the circle.
//!
//! On the upper half of the fitted view the text reads left to right with its
//! tops toward the rim; on the lower half it flips, so it reads left to right
//! there too instead of upside down.
//!
//! Labels step aside, along their arc, for [`ReservedArcs`]: parts of the
//! ring something else occupies at the fitted view, like the window buttons.
//!
//! Text is rasterised at its font size whatever the zoom, so a label zoomed in
//! would blur: its glyphs are rasterised larger and scaled back down, by a
//! power of two that follows how many pixels a circle unit covers. Any other
//! `Text2d` inside the circle gets the same treatment from [`Sharp`].

use std::f32::consts::FRAC_PI_2;

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::camera::{CAKE_LAYER, Rig, px_per_unit};
use crate::ringmesh::wrap_pi;

/// Where players' names run: on the ring beyond the map.
pub const NAME_RADIUS: f32 = 526.0;
pub const NAME_SIZE: f32 = 16.0;

/// Advance of the built-in monospace font, as a share of its size.
const ADVANCE: f32 = 0.6;
/// Clearance between a label and a reserved arc, in radians.
const MARGIN: f32 = 0.026;
/// The largest factor glyphs are rasterised at: at the closest zoom a label
/// is still sharp, and the font atlas holds a few sizes per label at most.
const MAX_RASTER: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Frame {
    /// The angle is on the map: the label turns with the view.
    #[default]
    Map,
    /// The angle is the screen's: the label does not turn with the view.
    Screen,
}

/// A line of text laid along an arc. Its glyphs are children of this entity,
/// respawned when the text or size changes, or the zoom wants them
/// rasterised at another scale; angle, radius and colour just move and tint
/// them, so a label can slide and fade every frame.
#[derive(Component, Clone, Debug, PartialEq)]
#[require(Transform, Visibility)]
pub struct ArcText {
    pub text: String,
    /// Where the text is centred, in radians, in `frame`.
    pub angle: f32,
    pub frame: Frame,
    /// The radius of the text's middle line, in circle units.
    pub radius: f32,
    /// Font size, in circle units.
    pub size: f32,
    pub color: Color,
}

impl ArcText {
    /// A player's name on the ring beyond the map, beside map angle `angle`.
    pub fn name(text: String, angle: f32, color: Color) -> ArcText {
        ArcText {
            text,
            angle,
            frame: Frame::Map,
            radius: NAME_RADIUS,
            size: NAME_SIZE,
            color,
        }
    }

    /// Half the angle the text spans.
    fn half_width(&self) -> f32 {
        self.text.chars().count() as f32 * self.size * ADVANCE / 2.0 / self.radius
    }
}

/// A stretch of the ring that labels keep off: screen angles `span`, between
/// radii `inner` and `outer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reserved {
    pub span: (f32, f32),
    pub inner: f32,
    pub outer: f32,
}

#[derive(Resource, Default, Debug, PartialEq)]
pub struct ReservedArcs(pub Vec<Reserved>);

#[derive(Component)]
struct Glyph(usize);

/// What the glyphs were built from.
#[derive(Component, PartialEq)]
struct Built {
    text: String,
    size: f32,
    /// The glyphs are rasterised at this many times `size`, and scaled back.
    raster: f32,
}

/// A `Text2d` inside the circle, of this font size in circle units, kept
/// sharp as the view zooms: its font is set to the size times the raster
/// factor, and its transform's scale to the inverse.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Sharp(pub f32);

pub fn plugin(app: &mut App) {
    app.init_resource::<ReservedArcs>()
        .add_systems(Update, (sharpen, rebuild, place).chain());
}

/// How many times their size the circle's texts are rasterised at now.
fn raster_now(rig: &Rig, window: Option<&Window>) -> f32 {
    raster(window.map_or(1.0, px_per_unit) / rig.zoom)
}

/// How many times its size to rasterise text at, when one of its circle
/// units covers `px` pixels: the nearest power of two, from 1 to
/// [`MAX_RASTER`], so zooming re-rasterises only now and then.
fn raster(px: f32) -> f32 {
    px.max(1.0).log2().round().exp2().min(MAX_RASTER)
}

/// Respawn the glyphs of every label whose text or size changed, or
/// whose glyphs are due to be rasterised at another scale.
#[allow(clippy::type_complexity)]
fn rebuild(
    mut commands: Commands,
    rig: Res<Rig>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    labels: Query<(Entity, &ArcText, Option<&Built>, Option<&Children>)>,
) {
    let raster = raster_now(&rig, window.as_ref().map(|w| **w));
    for (entity, label, built, children) in &labels {
        if built.is_some_and(|b| b.text == label.text && b.size == label.size && b.raster == raster)
        {
            continue;
        }
        commands.entity(entity).insert(Built {
            text: label.text.clone(),
            size: label.size,
            raster,
        });
        if let Some(children) = children {
            for c in children.iter() {
                commands.entity(c).despawn();
            }
        }
        for (i, ch) in label.text.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            commands.spawn((
                Text2d::new(ch.to_string()),
                TextFont {
                    font_size: bevy::text::FontSize::Px(label.size * raster),
                    ..default()
                },
                TextColor(label.color),
                Glyph(i),
                RenderLayers::layer(CAKE_LAYER),
                ChildOf(entity),
            ));
        }
    }
}

/// Rasterise every [`Sharp`] text at the scale the zoom wants now.
fn sharpen(
    rig: Res<Rig>,
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut texts: Query<(&Sharp, &mut TextFont, &mut Transform)>,
) {
    let raster = raster_now(&rig, window.as_ref().map(|w| **w));
    for (sharp, mut font, mut tf) in &mut texts {
        let size = bevy::text::FontSize::Px(sharp.0 * raster);
        if font.font_size != size {
            font.font_size = size;
        }
        let scale = Vec3::splat(raster.recip());
        if tf.scale != scale {
            tf.scale = scale;
        }
    }
}

/// Move a label centred at screen angle `angle`, `half` radians to each side,
/// off whichever reserved arc it overlaps, to that arc's nearer side.
fn step_aside(angle: f32, half: f32, radius: f32, reserved: &[Reserved]) -> f32 {
    for r in reserved {
        if !(r.inner..=r.outer).contains(&radius) {
            continue;
        }
        let (from, to) = r.span;
        let centre = (from + to) / 2.0;
        let a = centre + wrap_pi(angle - centre);
        if a + half < from - MARGIN || a - half > to + MARGIN {
            continue;
        }
        return if a < centre {
            from - MARGIN - half
        } else {
            to + MARGIN + half
        };
    }
    angle
}

/// Where the `i`-th of `n` glyphs goes, and how it is turned, for a label
/// centred at screen angle `angle`.
pub fn glyph_transform(i: usize, n: usize, angle: f32, radius: f32, size: f32) -> (Vec2, f32) {
    // Upper half of the screen: read clockwise, tops outward. Lower half:
    // read counter-clockwise, tops inward.
    let upper = angle.sin() >= -0.05;
    let step = size * ADVANCE / radius;
    let offset = (i as f32 - (n as f32 - 1.0) / 2.0) * step;
    let a = if upper {
        angle - offset
    } else {
        angle + offset
    };
    let turn = if upper { a - FRAC_PI_2 } else { a + FRAC_PI_2 };
    (Vec2::from_angle(a) * radius, turn)
}

/// Lay each label's glyphs along its arc. A label on the circle is laid out
/// as the fitted view shows it; the cake camera takes it from there.
fn place(
    rig: Res<Rig>,
    reserved: Res<ReservedArcs>,
    labels: Query<(&ArcText, &Built, &Children)>,
    mut glyphs: Query<(&Glyph, &mut Transform, &mut TextColor)>,
) {
    for (label, built, children) in &labels {
        let n = label.text.chars().count();
        let on_screen = match label.frame {
            Frame::Map => label.angle - rig.rotation,
            Frame::Screen => label.angle,
        };
        let angle = step_aside(on_screen, label.half_width(), label.radius, &reserved.0);
        for c in children.iter() {
            let Ok((glyph, mut tf, mut color)) = glyphs.get_mut(c) else {
                continue;
            };
            color.set_if_neq(TextColor(label.color));
            let (at, turn) = glyph_transform(glyph.0, n, angle, label.radius, label.size);
            tf.set_if_neq(
                Transform::from_translation(at.extend(1.0))
                    .with_rotation(Quat::from_rotation_z(turn))
                    .with_scale(Vec3::splat(built.raster.recip())),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// At the top, glyphs run left to right and stand upright.
    #[test]
    fn text_at_the_top_reads_left_to_right_and_upright() {
        let (first, turn) = glyph_transform(0, 5, FRAC_PI_2, 380.0, 15.0);
        let (last, _) = glyph_transform(4, 5, FRAC_PI_2, 380.0, 15.0);
        assert!(first.x < last.x, "{first} {last}");
        let (_, middle_turn) = glyph_transform(2, 5, FRAC_PI_2, 380.0, 15.0);
        assert!(middle_turn.abs() < 1e-5, "the middle glyph stands upright");
        assert!(turn > 0.0, "the first glyph leans left, following the arc");
    }

    /// At the bottom it flips: still left to right, and upright, not upside
    /// down.
    #[test]
    fn text_at_the_bottom_flips_to_stay_readable() {
        let (first, _) = glyph_transform(0, 5, -FRAC_PI_2, 380.0, 15.0);
        let (last, _) = glyph_transform(4, 5, -FRAC_PI_2, 380.0, 15.0);
        assert!(first.x < last.x, "{first} {last}");
        let (_, turn) = glyph_transform(2, 5, -FRAC_PI_2, 380.0, 15.0);
        assert!(wrap_pi(turn).abs() < 1e-5);
    }

    #[test]
    fn text_is_rasterised_at_the_nearest_power_of_two_up_to_the_cap() {
        assert_eq!(raster(0.3), 1.0, "shrunk text is never rasterised smaller");
        assert_eq!(raster(1.3), 1.0);
        assert_eq!(raster(1.5), 2.0);
        assert_eq!(raster(4.6), 4.0);
        assert_eq!(raster(6.0), 8.0);
        assert_eq!(raster(100.0), MAX_RASTER);
    }

    #[test]
    fn glyphs_sit_on_the_arc() {
        for i in 0..7 {
            let (p, _) = glyph_transform(i, 7, 1.0, 380.0, 15.0);
            assert!((p.length() - 380.0).abs() < 1e-3);
        }
    }

    #[test]
    fn labels_step_aside_for_reserved_arcs_on_their_radius() {
        let reserved = [Reserved {
            span: (1.3, 1.85),
            inner: 512.0,
            outer: 600.0,
        }];
        let half = 0.07;
        // On the reserved arc: moved clear of it, to the nearer side.
        let moved = step_aside(1.6, half, 526.0, &reserved);
        assert!(moved - half > 1.85, "{moved}");
        let moved = step_aside(1.5, half, 526.0, &reserved);
        assert!(moved + half < 1.3, "{moved}");
        // Elsewhere, or on another radius: left alone.
        assert_eq!(step_aside(-FRAC_PI_2, half, 526.0, &reserved), -FRAC_PI_2);
        assert_eq!(step_aside(1.6, half, 380.0, &reserved), 1.6);
        // Across the seam the arc is still found.
        let wrapped = step_aside(1.6 - 2.0 * PI, half, 526.0, &reserved);
        assert!(wrapped - half > 1.85 || wrapped + half < 1.3);
    }
}
