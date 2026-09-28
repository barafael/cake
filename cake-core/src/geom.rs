//! Ring geometry in integer polar coordinates.
//!
//! The map is an annulus centred on the origin, and the simulation uses that
//! centre as its coordinate system directly: a position is an angle and a
//! radius. Everything is an integer, so every peer computes bit-identical
//! results whatever its platform: there are no floats and no trigonometry in
//! the simulation.
//!
//! - [`Angle`] is a `u32` where 2³² is one full turn, so wrap-around is the
//!   integer's own overflow.
//! - Lengths are `i64` milli-units: [`UNIT`] is one map unit.
//!
//! Distances use the local tangent frame at the mean radius of the two points:
//! `d² = (r̄·Δθ)² + Δr²`. The band is thin and interesting ranges are short, so
//! this is the arc distance along the band, which is what movement follows.

use serde::{Deserialize, Serialize};

/// One map unit, in the simulation's milli-units.
pub const UNIT: i64 = 1000;
pub const R_INNER: i64 = 400 * UNIT;
pub const R_OUTER: i64 = 500 * UNIT;
pub const R_MID: i64 = 450 * UNIT;

/// τ as a rational, for the angle <-> arc-length conversions. Nine digits is
/// far below a milli-unit of error around the whole ring.
const TAU_NUM: i128 = 6_283_185_307;
const TAU_DEN: i128 = 1_000_000_000;

/// Angle units per milli-unit of arc at the inner edge, rounded up: the most
/// angle any length can span in the band. Used to reject far pairs before the
/// exact distance.
const ANGLE_PER_MILLI_AT_INNER: i64 = 1710;

/// An angle, with one full turn = 2³². Zero is the +x axis, and positive
/// deltas turn counter-clockwise.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
pub struct Angle(pub u32);

impl Angle {
    /// `num/den` of a full turn.
    pub fn from_turns(num: u64, den: u64) -> Angle {
        Angle(((num % den) * (1u64 << 32) / den) as u32)
    }

    /// Signed shortest difference `self - from`, in angle units.
    pub fn delta(self, from: Angle) -> i64 {
        self.0.wrapping_sub(from.0) as i32 as i64
    }

    /// Turn by `d` angle units; any `i64` wraps correctly.
    pub fn turned(self, d: i64) -> Angle {
        Angle(self.0.wrapping_add(d as u32))
    }

    /// Presentation only: radians, for turning the sim into pixels. Never
    /// feed this back into the simulation.
    pub fn to_radians(self) -> f64 {
        self.0 as f64 / (1u64 << 32) as f64 * std::f64::consts::TAU
    }

    /// Presentation only: the angle nearest to `radians`.
    pub fn from_radians(radians: f64) -> Angle {
        let turns = radians / std::f64::consts::TAU;
        let frac = turns - turns.floor();
        Angle((frac * (1u64 << 32) as f64) as u64 as u32)
    }
}

/// Arc length, in milli-units, that `dtheta` angle units span at radius `r`.
pub fn arc_len(dtheta: i64, r: i64) -> i64 {
    (dtheta as i128 * r as i128 * TAU_NUM / (TAU_DEN << 32)) as i64
}

/// Angle units that an arc of `len` milli-units spans at radius `r`.
pub fn arc_to_angle(len: i64, r: i64) -> i64 {
    (((len as i128) << 32) * TAU_DEN / (r.max(1) as i128 * TAU_NUM)) as i64
}

/// A point on the map.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Pos {
    pub a: Angle,
    /// Radius in milli-units.
    pub r: i64,
}

impl Pos {
    /// A point in the band: `r` is clamped to it.
    pub fn new(a: Angle, r: i64) -> Pos {
        Pos {
            a,
            r: r.clamp(R_INNER, R_OUTER),
        }
    }

    /// This point pulled at least `margin` inside both edges of the band.
    pub fn inset(self, margin: i64) -> Pos {
        Pos {
            a: self.a,
            r: self.r.clamp(R_INNER + margin, R_OUTER - margin),
        }
    }

    /// Offset from `self` to `other` as `(tangential, radial)` milli-units.
    /// Tangential is positive counter-clockwise, and radial is positive
    /// outward.
    pub fn offset_to(self, other: Pos) -> (i64, i64) {
        let rbar = (self.r + other.r) / 2;
        (arc_len(other.a.delta(self.a), rbar), other.r - self.r)
    }

    pub fn dist2(self, other: Pos) -> i64 {
        let (t, r) = self.offset_to(other);
        t * t + r * r
    }

    pub fn dist(self, other: Pos) -> i64 {
        isqrt(self.dist2(other))
    }

    /// Is `other` within `range` of `self`? Far pairs are rejected on the
    /// angle alone, which is most pairs on a ring.
    pub fn within(self, other: Pos, range: i64) -> bool {
        if other.a.delta(self.a).abs() > range.saturating_mul(ANGLE_PER_MILLI_AT_INNER) {
            return false;
        }
        self.dist2(other) <= range * range
    }

    /// Move by `(tangential, radial)` milli-units, measured at this radius.
    /// The radius is clamped to the band.
    pub fn displaced(self, dt: i64, dr: i64) -> Pos {
        Pos::new(self.a.turned(arc_to_angle(dt, self.r)), self.r + dr)
    }

    /// Step toward `target` by at most `step` milli-units. Returns the new
    /// point and whether it reached `target`.
    pub fn step_toward(self, target: Pos, step: i64) -> (Pos, bool) {
        let (t, r) = self.offset_to(target);
        let d2 = t * t + r * r;
        if d2 <= step * step {
            return (target, true);
        }
        let d = isqrt(d2);
        (self.displaced(t * step / d, r * step / d), false)
    }

    /// Presentation only: Cartesian map units, origin at the ring's centre.
    pub fn to_xy(self) -> (f64, f64) {
        let theta = self.a.to_radians();
        let r = self.r as f64 / UNIT as f64;
        (r * theta.cos(), r * theta.sin())
    }

    /// Presentation only: the point at Cartesian map units `(x, y)`, clamped
    /// into the band.
    pub fn from_xy(x: f64, y: f64) -> Pos {
        let r = (x * x + y * y).sqrt() * UNIT as f64;
        Pos::new(Angle::from_radians(y.atan2(x)), r as i64)
    }
}

/// Integer square root (floor) of a non-negative `i64`.
pub fn isqrt(v: i64) -> i64 {
    (v.max(0) as u64).isqrt() as i64
}

/// Scale `(x, y)` to length `len`. The zero vector stays zero.
pub fn scaled(x: i64, y: i64, len: i64) -> (i64, i64) {
    let d = isqrt(x * x + y * y);
    if d == 0 {
        (0, 0)
    } else {
        (x * len / d, y * len / d)
    }
}

/// The centre angle of seat `i` of `n` around the ring.
pub fn sector_center(i: usize, n: usize) -> Angle {
    Angle::from_turns(i as u64, n.max(1) as u64)
}

/// The boundary between seat `i` and seat `i + 1` of `n`.
pub fn sector_edge(i: usize, n: usize) -> Angle {
    Angle::from_turns(2 * i as u64 + 1, 2 * n.max(1) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_wrap_through_zero() {
        let a = Angle(u32::MAX - 10);
        let b = Angle(10);
        assert_eq!(b.delta(a), 21);
        assert_eq!(a.delta(b), -21);
        assert_eq!(a.turned(21), b);
        assert_eq!(b.turned(-21), a);
    }

    #[test]
    fn half_turn_is_the_extreme_delta() {
        let half = Angle::from_turns(1, 2);
        assert_eq!(half.0, 1 << 31);
        assert_eq!(half.delta(Angle(0)).abs(), 1 << 31);
    }

    #[test]
    fn arc_length_matches_the_circumference() {
        // A quarter turn at the mid radius: τ·450/4 ≈ 706.858 units.
        let quarter = Angle::from_turns(1, 4).0 as i64;
        let len = arc_len(quarter, R_MID);
        assert!((len - 706_858).abs() <= 1, "{len}");
        let back = arc_to_angle(len, R_MID);
        assert!((back - quarter).abs() < 2000, "{back} vs {quarter}");
    }

    #[test]
    fn distance_across_the_seam_is_short() {
        let a = Pos::new(Angle(u32::MAX - 1000), R_MID);
        let b = Pos::new(Angle(1000), R_MID);
        // 2001 angle units at radius 450 is about 1.3 milli-units.
        assert!(a.dist(b) < 5);
        assert!(a.within(b, UNIT));
    }

    #[test]
    fn radial_distance_is_exact() {
        let a = Pos::new(Angle(123), R_INNER);
        let b = Pos::new(Angle(123), R_OUTER);
        assert_eq!(a.dist(b), 100 * UNIT);
        assert!(a.within(b, 100 * UNIT));
        assert!(!a.within(b, 100 * UNIT - 1));
    }

    #[test]
    fn stepping_reaches_the_target_and_never_leaves_the_band() {
        let mut p = Pos::new(Angle(0), R_INNER);
        let target = Pos::new(Angle::from_turns(1, 16), R_OUTER);
        let mut steps = 0;
        loop {
            let (next, arrived) = p.step_toward(target, 2 * UNIT);
            assert!((R_INNER..=R_OUTER).contains(&next.r));
            p = next;
            steps += 1;
            if arrived {
                break;
            }
            assert!(steps < 10_000, "never arrived");
        }
        assert_eq!(p, target);
    }

    #[test]
    fn stepping_takes_the_short_way_round() {
        let p = Pos::new(Angle::from_turns(15, 16), R_MID);
        let target = Pos::new(Angle::from_turns(1, 16), R_MID);
        let (next, _) = p.step_toward(target, UNIT);
        assert!(
            next.a.delta(p.a) > 0,
            "should turn counter-clockwise through zero"
        );
    }

    #[test]
    fn displacement_clamps_radius() {
        let p = Pos::new(Angle(0), R_OUTER - UNIT);
        assert_eq!(p.displaced(0, 10 * UNIT).r, R_OUTER);
    }

    #[test]
    fn sectors_are_evenly_spaced() {
        // Counter-clockwise gaps as unsigned turns: with two seats the gap is
        // exactly half a turn, which has no signed direction.
        let gap = |to: Angle, from: Angle| to.0.wrapping_sub(from.0) as i64;
        for n in 2..=8 {
            let step = gap(sector_center(1, n), sector_center(0, n));
            for i in 0..n {
                let d = gap(sector_center((i + 1) % n, n), sector_center(i, n));
                assert!((d - step).abs() <= 1, "n={n} i={i}");
                let e = gap(sector_edge(i, n), sector_center(i, n));
                assert!((2 * e - step).abs() <= 2, "n={n} i={i}");
            }
        }
    }

    #[test]
    fn xy_round_trips() {
        let p = Pos::new(Angle::from_turns(3, 7), 432 * UNIT);
        let (x, y) = p.to_xy();
        let q = Pos::from_xy(x, y);
        assert!(p.dist(q) < 5, "{p:?} {q:?}");
    }
}
