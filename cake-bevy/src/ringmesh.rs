//! Ring geometry for drawing: angles, arcs, rows of slots, and meshes for
//! pieces of rings.
//!
//! Angles are radians, counter-clockwise from +x. Bevy's `Annulus` maps its
//! texture around the ring; the meshes here map it flat: with `uv_radius` R,
//! UV (0, 0) is the top-left corner of the 2R x 2R square centred on the
//! origin, exactly as a sprite of that size would show the texture, so a ring
//! textured this way lines up seamlessly with such a sprite.

use std::f32::consts::{PI, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// `a` wrapped into `(-PI, PI]`.
pub fn wrap_pi(a: f32) -> f32 {
    let w = (a + PI).rem_euclid(TAU) - PI;
    if w == -PI { PI } else { w }
}

/// The angle of point `p`.
pub fn angle_of(p: Vec2) -> f32 {
    p.y.atan2(p.x)
}

/// Is angle `a` within the span `(from, to)` (`from <= to`, any turn)?
pub fn in_span(a: f32, (from, to): (f32, f32)) -> bool {
    let d = (a - from).rem_euclid(TAU);
    d <= to - from
}

/// Is `p` inside the ring between radii `inner` and `outer`, within `span`?
pub fn sector_contains(p: Vec2, inner: f32, outer: f32, span: (f32, f32)) -> bool {
    (inner..=outer).contains(&p.length()) && in_span(angle_of(p), span)
}

/// How many straight pieces make `sweep` radians of arc look round.
fn pieces(sweep: f32) -> usize {
    ((sweep.abs() / TAU) * 512.0).ceil().max(2.0) as usize
}

/// Points along the arc of `radius` from `from` through `sweep` radians
/// (either sign), both ends included.
pub fn arc(radius: f32, from: f32, sweep: f32) -> impl Iterator<Item = Vec2> {
    let n = pieces(sweep);
    (0..=n).map(move |k| Vec2::from_angle(from + sweep * k as f32 / n as f32) * radius)
}

/// A row of equal slots side by side along the ring, like buttons on an arc.
#[derive(Clone, Copy, Debug)]
pub struct Slots {
    pub inner: f32,
    pub outer: f32,
    /// Where the row is centred, in degrees.
    pub centre: f32,
    /// Each slot's width and the gap between two, in degrees.
    pub width: f32,
    pub gap: f32,
    /// Slot 0 is the first clockwise (true) or counter-clockwise. Across the
    /// top of the screen clockwise runs left to right; across the bottom,
    /// counter-clockwise does.
    pub clockwise: bool,
}

impl Slots {
    /// Slot `i` of `n`, as an angle span `(from, to)`.
    pub fn span(&self, i: usize, n: usize) -> (f32, f32) {
        let pitch = self.width + self.gap;
        let offset = (i as f32 - (n as f32 - 1.0) / 2.0) * pitch;
        let centre = if self.clockwise {
            self.centre - offset
        } else {
            self.centre + offset
        };
        (
            (centre - self.width / 2.0).to_radians(),
            (centre + self.width / 2.0).to_radians(),
        )
    }

    /// The middle of slot `i` of `n`.
    pub fn centre_of(&self, i: usize, n: usize) -> Vec2 {
        let (from, to) = self.span(i, n);
        Vec2::from_angle((from + to) / 2.0) * (self.inner + self.outer) / 2.0
    }

    /// Which of `n` slots holds `p`.
    pub fn at(&self, p: Vec2, n: usize) -> Option<usize> {
        (0..n).find(|&i| sector_contains(p, self.inner, self.outer, self.span(i, n)))
    }

    /// The whole row, as one angle span.
    pub fn extent(&self, n: usize) -> (f32, f32) {
        let (a, b) = (self.span(0, n), self.span(n.saturating_sub(1), n));
        (a.0.min(b.0), a.1.max(b.1))
    }

    /// A mesh for slot `i` of `n`.
    pub fn mesh(&self, i: usize, n: usize, uv_radius: f32) -> Mesh {
        let (from, to) = self.span(i, n);
        ring_sector(self.inner, self.outer, from, to, uv_radius)
    }
}

/// The part of the ring between `inner` and `outer` radii, from angle `from`
/// to `to`.
pub fn ring_sector(inner: f32, outer: f32, from: f32, to: f32, uv_radius: f32) -> Mesh {
    let sweep = to - from;
    let mut positions = Vec::new();
    let mut uvs = Vec::new();
    for (i, o) in arc(inner, from, sweep).zip(arc(outer, from, sweep)) {
        for p in [i, o] {
            positions.push([p.x, p.y, 0.0]);
            uvs.push([0.5 + p.x / (2.0 * uv_radius), 0.5 - p.y / (2.0 * uv_radius)]);
        }
    }
    let n = pieces(sweep) as u32;
    let mut indices = Vec::with_capacity(n as usize * 6);
    for i in 0..n {
        let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
        // Counter-clockwise, whichever way the sector runs.
        if sweep >= 0.0 {
            indices.extend([a, b, c, b, d, c]);
        } else {
            indices.extend([a, c, b, b, c, d]);
        }
    }
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// A whole ring.
pub fn ring(inner: f32, outer: f32, uv_radius: f32) -> Mesh {
    ring_sector(inner, outer, 0.0, TAU, uv_radius)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    #[test]
    fn uvs_match_a_sprite_of_the_uv_square() {
        let mesh = ring_sector(10.0, 20.0, 0.0, std::f32::consts::FRAC_PI_2, 20.0);
        let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!("uvs");
        };
        // The first outer vertex is at (20, 0): the middle of the right edge.
        assert_eq!(uvs[1], [1.0, 0.5]);
        // The last outer vertex is at (0, 20): the middle of the top edge.
        let last = uvs[uvs.len() - 1];
        assert!(
            (last[0] - 0.5).abs() < 1e-5 && last[1].abs() < 1e-5,
            "{last:?}"
        );
    }

    #[test]
    fn triangles_wind_counter_clockwise() {
        for (from, to) in [(0.0, 1.0), (1.0, 0.0)] {
            let mesh = ring_sector(10.0, 20.0, from, to, 20.0);
            let Some(VertexAttributeValues::Float32x3(p)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("positions");
            };
            let Some(Indices::U32(idx)) = mesh.indices() else {
                panic!("indices");
            };
            for t in idx.chunks(3) {
                let [a, b, c] =
                    [t[0], t[1], t[2]].map(|i| Vec2::new(p[i as usize][0], p[i as usize][1]));
                assert!((b - a).perp_dot(c - a) > 0.0, "clockwise triangle {t:?}");
            }
        }
    }

    #[test]
    fn spans_hold_angles_across_the_seam() {
        let span = (170f32.to_radians(), 200f32.to_radians());
        assert!(in_span(180f32.to_radians(), span));
        assert!(
            in_span(-170f32.to_radians(), span),
            "atan2's side of the seam"
        );
        assert!(!in_span(0.0, span));
        assert!((wrap_pi(3.0 * PI) - PI).abs() < 1e-5);
        assert!((wrap_pi(-0.5) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn slots_run_the_way_they_are_told() {
        let top = Slots {
            inner: 10.0,
            outer: 20.0,
            centre: 90.0,
            width: 10.0,
            gap: 2.0,
            clockwise: true,
        };
        let bottom = Slots {
            centre: 270.0,
            clockwise: false,
            ..top
        };
        for row in [top, bottom] {
            for i in 0..3 {
                assert_eq!(row.at(row.centre_of(i, 3), 3), Some(i));
                if i < 2 {
                    assert!(
                        row.centre_of(i, 3).x < row.centre_of(i + 1, 3).x,
                        "left to right"
                    );
                }
            }
        }
    }
}
