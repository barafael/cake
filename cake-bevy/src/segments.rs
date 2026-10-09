//! Segment buttons: slices of a ring that say what they do along their arc.
//!
//! Everything in cake is circular, and so are its buttons. A [`Segment`] is
//! slot `index` of a [`Slots`] row: a mesh for its fill, a title along the
//! arc, and optionally a detail (a key, a price) under it. The pointer lights
//! the segment it is over; a press on an enabled one sends [`SegmentPressed`]
//! with the segment's entity, and whoever spawned it acts on its own
//! component there. Segments claim the pointer, so the map never sees a click
//! meant for one.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};

use crate::arctext::{ArcText, Frame};
use crate::camera::{CAKE_LAYER, Cursor};
use crate::chrome::{PointerClaimed, PointerSet, RADIUS, block_pointer};
use crate::ringmesh::{Slots, sector_contains};
use crate::{AppState, palette};

/// A segment button.
#[derive(Component, Clone, Copy, Debug)]
pub struct Segment {
    pub row: Slots,
    pub index: usize,
    pub count: usize,
    /// A disabled segment is drawn faint and ignores presses.
    pub enabled: bool,
    /// Lit: the chosen one of a row of choices.
    pub lit: bool,
}

impl Segment {
    fn span(&self) -> (f32, f32) {
        self.row.span(self.index, self.count)
    }

    fn contains(&self, p: Vec2) -> bool {
        sector_contains(p, self.row.inner, self.row.outer, self.span())
    }
}

/// A segment was clicked.
#[derive(Message, Clone, Copy, Debug)]
pub struct SegmentPressed(pub Entity);

/// The segment under the pointer, if any.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct SegmentHover(pub Option<Entity>);

/// A segment's label (its title, or its detail), and its colour while the
/// segment is enabled.
#[derive(Component)]
pub struct SegmentLabel {
    segment: Entity,
    detail: bool,
    ink: Color,
}

/// Segment fills, shared by every segment.
#[derive(Resource)]
pub struct SegmentFills {
    idle: Handle<ColorMaterial>,
    hover: Handle<ColorMaterial>,
    off: Handle<ColorMaterial>,
    lit: Handle<ColorMaterial>,
}

impl FromWorld for SegmentFills {
    fn from_world(world: &mut World) -> Self {
        let mut materials = world.resource_mut::<Assets<ColorMaterial>>();
        SegmentFills {
            idle: materials.add(ColorMaterial::from_color(palette::CONTROL_IDLE)),
            hover: materials.add(ColorMaterial::from_color(palette::CONTROL_HOVER)),
            off: materials.add(ColorMaterial::from_color(palette::CONTROL_OFF)),
            lit: materials.add(ColorMaterial::from_color(palette::CONTROL_CHOSEN)),
        }
    }
}

pub fn plugin(app: &mut App) {
    app.add_message::<SegmentPressed>()
        .init_resource::<SegmentHover>()
        .init_resource::<SegmentFills>()
        .add_systems(Update, hover.in_set(PointerSet).before(block_pointer))
        .add_systems(Update, (press, shade).chain().after(PointerSet));
}

/// Where a segment's title and detail run: the title nearer the centre, the
/// detail nearer the rim. At the bottom of the screen the text is flipped,
/// tops inward, so the title reads above the detail there; at the top the
/// detail is on the inside of the title.
fn label_radii(row: &Slots, bottom: bool) -> (f32, f32) {
    let (near_centre, near_rim) = (row.inner + 19.0, row.outer - 13.0);
    if bottom {
        (near_centre, near_rim)
    } else {
        (near_rim - 4.0, near_centre - 2.0)
    }
}

/// Spawn slot `index` of `count` in `row`, titled `title`, with an optional
/// `detail`, carrying `extra` (what it does), despawned when `state` ends.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    fills: &SegmentFills,
    row: Slots,
    index: usize,
    count: usize,
    title: &str,
    detail: &str,
    state: AppState,
    extra: impl Bundle,
) -> Entity {
    let segment = Segment {
        row,
        index,
        count,
        enabled: true,
        lit: false,
    };
    let (from, to) = segment.span();
    let angle = (from + to) / 2.0;
    let e = commands
        .spawn((
            Mesh2d(meshes.add(row.mesh(index, count, RADIUS))),
            MeshMaterial2d(fills.idle.clone()),
            Transform::from_xyz(0.0, 0.0, -4.0),
            RenderLayers::layer(CAKE_LAYER),
            segment,
            DespawnOnExit(state),
            extra,
        ))
        .id();
    let (title_radius, detail_radius) = label_radii(&row, angle.sin() < 0.0);
    for (text, detail, radius, size, ink) in [
        (title, false, title_radius, 15.0, palette::TEXT),
        (detail, true, detail_radius, 12.0, palette::DIM_TEXT),
    ] {
        commands.spawn((
            ArcText {
                text: text.to_string(),
                angle,
                frame: Frame::Screen,
                radius,
                size,
                color: ink,
            },
            SegmentLabel {
                segment: e,
                detail,
                ink,
            },
            ChildOf(e),
        ));
    }
    e
}

/// Change a segment's title and detail in place.
pub fn relabel(
    labels: &mut Query<(&SegmentLabel, &mut ArcText)>,
    segment: Entity,
    title: &str,
    detail: &str,
) {
    for (label, mut arc) in labels.iter_mut() {
        if label.segment != segment {
            continue;
        }
        let text = if label.detail { detail } else { title };
        if arc.text != text {
            arc.text = text.to_string();
        }
    }
}

fn hover(
    cursor: Res<Cursor>,
    segments: Query<(Entity, &Segment, &InheritedVisibility)>,
    mut hover: ResMut<SegmentHover>,
    mut claimed: ResMut<PointerClaimed>,
) {
    let over = cursor.cake.and_then(|p| {
        segments
            .iter()
            .find(|(_, s, visible)| visible.get() && s.contains(p))
            .map(|(e, _, _)| e)
    });
    hover.set_if_neq(SegmentHover(over));
    if over.is_some() {
        claimed.0 = true;
    }
}

pub fn press(
    buttons: Res<ButtonInput<MouseButton>>,
    hover: Res<SegmentHover>,
    segments: Query<&Segment>,
    mut pressed: MessageWriter<SegmentPressed>,
) {
    if buttons.just_pressed(MouseButton::Left)
        && let Some(e) = hover.0
        && segments.get(e).is_ok_and(|s| s.enabled)
    {
        pressed.write(SegmentPressed(e));
    }
}

/// Fills for hover and availability; disabled segments' labels fade.
fn shade(
    hover: Res<SegmentHover>,
    fills: Res<SegmentFills>,
    mut segments: Query<(Entity, &Segment, &mut MeshMaterial2d<ColorMaterial>)>,
    mut labels: Query<(&SegmentLabel, &mut ArcText)>,
) {
    for (e, segment, mut material) in &mut segments {
        let fill = match (segment.enabled, hover.0 == Some(e)) {
            (false, _) => &fills.off,
            (true, true) => &fills.hover,
            (true, false) if segment.lit => &fills.lit,
            (true, false) => &fills.idle,
        };
        if material.0 != *fill {
            material.0 = fill.clone();
        }
    }
    for (label, mut arc) in &mut labels {
        let on = segments.get(label.segment).is_ok_and(|(_, s, _)| s.enabled);
        let color = if on { label.ink } else { palette::FAINT };
        if arc.color != color {
            arc.color = color;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_read_above_details_top_and_bottom() {
        let row = Slots {
            inner: 350.0,
            outer: 400.0,
            centre: 270.0,
            width: 15.0,
            gap: 1.0,
            clockwise: false,
        };
        // At the bottom, tops point inward: nearer the centre reads higher.
        let (title, detail) = label_radii(&row, true);
        assert!(title < detail);
        // At the top, tops point outward: nearer the rim reads higher.
        let (title, detail) = label_radii(&row, false);
        assert!(title > detail);
    }
}
