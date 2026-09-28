//! The recap: once a match is decided, the middle of the ring fills with a
//! chart of how it went.
//!
//! Time runs clockwise around the circle, from the top back round to it, and a
//! value is a radius, from the baseline out to the chart's rim. Every player is
//! a curve in their colour, ending in a cross where they were knocked out. The
//! segments along the bottom (or keys 1 to 5) pick what is shown, the pointer
//! over the chart reads off a moment, and the middle lists everyone's numbers
//! for it.

use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::sprite_render::{ColorMaterial, MeshMaterial2d};
use cake_core::history::{History, Metric};
use cake_core::stats::TICK_HZ;
use cake_core::{Outcome, Seat};

use crate::arctext::{ArcText, Frame};
use crate::camera::{Cursor, OVERLAY_LAYER};
use crate::chrome::RADIUS;
use crate::menu::{MENU_INNER, MENU_OUTER};
use crate::ringmesh::{self, Slots, wrap_pi};
use crate::segments::{self, Segment, SegmentFills, SegmentPressed};
use crate::{AppState, Match, palette};

/// The radius of a zero.
const BASE: f32 = 125.0;
/// The radius of the chart's top value.
const RIM: f32 = 285.0;
/// Left open at the top, between the start and the end, for the scale.
const GAP: f32 = 0.2;
/// The recap waits this long after the end, for the last explosions.
const DELAY: f32 = 1.2;
/// How long the curves take to draw themselves, first and on a new metric.
const SWEEP: f32 = 1.6;
const RESWEEP: f32 = 0.6;
const BANNER_RADIUS: f32 = 312.0;
/// Rows in the middle: one per player, at most eight.
const ROWS: usize = 8;

/// The metric choices, along the bottom of the inner ring.
const METRIC_ROW: Slots = Slots {
    inner: MENU_INNER,
    outer: MENU_OUTER,
    centre: 270.0,
    width: 16.0,
    gap: 1.0,
    clockwise: false,
};

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct RecapLines;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct RecapGlow;

pub fn plugin(app: &mut App) {
    app.init_gizmo_group::<RecapLines>()
        .init_gizmo_group::<RecapGlow>()
        .add_systems(Startup, configure_gizmos)
        .add_systems(OnExit(AppState::Game), |mut commands: Commands| {
            commands.remove_resource::<Recap>();
        })
        .add_systems(
            Update,
            (
                open,
                choose,
                (draw, darken, legend).run_if(resource_exists::<Recap>),
            )
                .chain()
                .after(segments::press)
                .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
        );
}

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (lines, _) = store.config_mut::<RecapLines>();
    lines.render_layers = RenderLayers::layer(OVERLAY_LAYER);
    lines.line.width = 2.0;
    // Mitred, so a wide curve has no notches at its bends.
    lines.line.joints = GizmoLineJoint::Miter;
    let (glow, _) = store.config_mut::<RecapGlow>();
    glow.render_layers = RenderLayers::layer(OVERLAY_LAYER);
    glow.line.width = 7.0;
    glow.line.joints = GizmoLineJoint::Miter;
}

/// Present, the recap moves on to the next metric by itself after this many
/// seconds on each: for demos, with no hand on the keys.
#[derive(Resource, Clone, Copy, Debug)]
pub struct AutoCycle(pub f32);

#[derive(Resource)]
struct Recap {
    metric: Metric,
    /// When the curves start drawing, in app seconds.
    since: f32,
    /// How long they take.
    sweep: f32,
}

impl Recap {
    /// How far the curves are drawn, 0 to 1.
    fn drawn(&self, now: f32) -> f32 {
        let t = ((now - self.since) / self.sweep).clamp(0.0, 1.0);
        1.0 - (1.0 - t).powi(3)
    }

    /// How far the chart has faded in, 0 to 1.
    fn shown(&self, now: f32) -> f32 {
        ((now - self.since) / 0.5).clamp(0.0, 1.0)
    }
}

/// A segment that shows a metric.
#[derive(Component)]
struct Choice(Metric);

/// A line of text in the middle, faded in with the chart.
#[derive(Component)]
enum Line {
    Heading,
    Caption,
    When,
    Row(usize),
    /// The value of gridline `i` (1 to 4), in the gap at the top.
    Scale(usize),
    /// A time along the rim, which only fades in.
    Time,
}

/// The dark disc behind the chart.
#[derive(Component)]
struct Backdrop(Handle<ColorMaterial>);

/// How it ended, in a line, or `None` while it goes on.
pub fn verdict(m: &Match) -> Option<(String, Color)> {
    match m.sim.outcome? {
        Outcome::Winner(w) if Some(w) == m.me => {
            Some(("Victory!".into(), palette::seat(w as usize)))
        }
        Outcome::Winner(w) => Some((
            format!("{} holds the ring", m.players[w as usize].name),
            palette::seat(w as usize),
        )),
        Outcome::Draw => Some(("Nobody is left".into(), palette::TEXT)),
    }
}

/// `tick` as minutes and seconds.
fn clock(tick: u32) -> String {
    let s = tick / TICK_HZ;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The screen angle of the moment `f` of the way through the match.
fn angle_at(f: f32) -> f32 {
    FRAC_PI_2 - GAP / 2.0 - (TAU - GAP) * f
}

/// The share of the match at screen angle `a`, if `a` is on the chart.
fn share_at(a: f32) -> Option<f32> {
    let f = (FRAC_PI_2 - GAP / 2.0 - a).rem_euclid(TAU) / (TAU - GAP);
    // A hair past the end is the end: the angle's rounding.
    (f <= 1.0 + 1e-4).then_some(f.min(1.0))
}

/// A round number at least `peak`, whose quarters are round too.
fn nice_top(peak: u32) -> u32 {
    for k in 0..9 {
        let base = 10u32.pow(k);
        for tenths in [10, 15, 20, 25, 30, 40, 50, 60, 80] {
            if (tenths * base) % 10 != 0 {
                continue;
            }
            let step = tenths * base / 10;
            if step * 4 >= peak.max(1) {
                return step * 4;
            }
        }
    }
    peak
}

/// Where a value sits: `v` of `top`, at moment `f`.
fn point(f: f32, v: f32, top: u32) -> Vec2 {
    let r = BASE + (RIM - BASE) * (v / top as f32).clamp(0.0, 1.0);
    Vec2::from_angle(angle_at(f)) * r
}

/// Samples either side that a drawn curve averages over: second-to-second
/// jitter hides the shape of a match. The numbers shown stay exact.
const SMOOTHING: usize = 2;

/// `seat`'s curve: each moment's share of the match and its value, smoothed,
/// up to the moment it was knocked out.
fn curve(history: &History, seat: usize, metric: Metric) -> Vec<(f32, f32)> {
    let end = history.end().max(1) as f32;
    let last = fell(history, seat).unwrap_or(u32::MAX);
    let raw: Vec<(f32, f32)> = history
        .samples
        .iter()
        .take_while(|s| s.tick <= last)
        .map(|s| (s.tick as f32 / end, s.players[seat].get(metric) as f32))
        .collect();
    smooth(&raw)
}

/// A moving average over [`SMOOTHING`] points either side, narrowing toward
/// the ends so that they keep their true values.
fn smooth(raw: &[(f32, f32)]) -> Vec<(f32, f32)> {
    (0..raw.len())
        .map(|i| {
            let k = SMOOTHING.min(i).min(raw.len() - 1 - i);
            let window = &raw[i - k..=i + k];
            let v = window.iter().map(|(_, v)| v).sum::<f32>() / window.len() as f32;
            (raw[i].0, v)
        })
        .collect()
}

/// The tick at which `seat` was knocked out, if it was.
fn fell(history: &History, seat: usize) -> Option<u32> {
    history
        .eliminated
        .iter()
        .find(|(s, _)| *s as usize == seat)
        .map(|(_, tick)| *tick)
}

/// The sample under the pointer, if it is over the chart.
fn hovered(cursor: &Cursor, history: &History) -> Option<usize> {
    let p = cursor.ui?;
    if !(BASE - 20.0..=RIM + 20.0).contains(&p.length()) {
        return None;
    }
    let f = share_at(p.to_angle())?;
    let tick = f * history.end() as f32;
    history
        .samples
        .iter()
        .enumerate()
        .min_by(|a, b| {
            let da = (a.1.tick as f32 - tick).abs();
            let db = (b.1.tick as f32 - tick).abs();
            da.total_cmp(&db)
        })
        .map(|(i, _)| i)
}

fn text(commands: &mut Commands, at: Vec2, size: f32, color: Color, line: Line) {
    commands.spawn((
        Text2d::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(size),
            ..default()
        },
        TextColor(color.with_alpha(0.0)),
        Transform::from_translation(at.extend(1.0)),
        RenderLayers::layer(OVERLAY_LAYER),
        DespawnOnExit(AppState::Game),
        line,
    ));
}

/// Once the match is decided: the metric choices, the lines of text, and the
/// disc behind it all.
#[allow(clippy::too_many_arguments)]
fn open(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    fills: Res<SegmentFills>,
    time: Res<Time>,
    m: Res<Match>,
    recap: Option<Res<Recap>>,
) {
    if recap.is_some() || m.sim.outcome.is_none() {
        return;
    }
    let metric = Metric::Army;
    commands.insert_resource(Recap {
        metric,
        since: time.elapsed_secs() + DELAY,
        sweep: SWEEP,
    });

    let fill = materials.add(ColorMaterial::from_color(Color::BLACK.with_alpha(0.0)));
    commands.spawn((
        Mesh2d(meshes.add(ringmesh::ring(0.0, RIM + 18.0, RADIUS))),
        MeshMaterial2d(fill.clone()),
        Transform::from_xyz(0.0, 0.0, -6.0),
        RenderLayers::layer(OVERLAY_LAYER),
        Backdrop(fill),
        DespawnOnExit(AppState::Game),
    ));

    for (i, metric) in Metric::ALL.into_iter().enumerate() {
        let key = (i + 1).to_string();
        segments::spawn(
            &mut commands,
            &mut meshes,
            &fills,
            METRIC_ROW,
            i,
            Metric::ALL.len(),
            metric.name(),
            &key,
            AppState::Game,
            Choice(metric),
        );
    }

    if let Some((line, color)) = verdict(&m) {
        commands.spawn((
            ArcText {
                text: line,
                angle: FRAC_PI_2,
                frame: Frame::Screen,
                radius: BANNER_RADIUS,
                size: 24.0,
                color,
            },
            DespawnOnExit(AppState::Game),
        ));
    }

    text(
        &mut commands,
        Vec2::new(0.0, 82.0),
        20.0,
        palette::TEXT,
        Line::Heading,
    );
    text(
        &mut commands,
        Vec2::new(0.0, 62.0),
        11.0,
        palette::DIM_TEXT,
        Line::Caption,
    );
    text(
        &mut commands,
        Vec2::new(0.0, 47.0),
        11.0,
        palette::DIM_TEXT,
        Line::When,
    );
    for i in 0..ROWS.min(m.players.len()) {
        let at = Vec2::new(0.0, 25.0 - 15.0 * i as f32);
        text(&mut commands, at, 13.0, palette::TEXT, Line::Row(i));
    }
    for i in 1..=4 {
        text(
            &mut commands,
            Vec2::ZERO,
            10.0,
            palette::DIM_TEXT,
            Line::Scale(i),
        );
    }

    // Times along the rim, clear of the verdict at the top.
    let end = m.history.end();
    let minutes = end / TICK_HZ / 60;
    let step = [1, 2, 5, 10, 15, 30, 60]
        .into_iter()
        .find(|s| minutes / s <= 10)
        .unwrap_or(60);
    for k in (step..=minutes).step_by(step as usize) {
        let f = (k * 60 * TICK_HZ) as f32 / end.max(1) as f32;
        let a = angle_at(f);
        if wrap_pi(a - FRAC_PI_2).abs() < 0.5 {
            continue;
        }
        let at = Vec2::from_angle(a) * (RIM + 15.0);
        commands.spawn((
            Text2d::new(format!("{k}m")),
            TextFont {
                font_size: bevy::text::FontSize::Px(10.0),
                ..default()
            },
            TextColor(palette::DIM_TEXT.with_alpha(0.0)),
            Transform::from_translation(at.extend(1.0)),
            RenderLayers::layer(OVERLAY_LAYER),
            DespawnOnExit(AppState::Game),
            Line::Time,
        ));
    }
}

/// Switch metrics from the segments or the number keys.
fn choose(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut pressed: MessageReader<SegmentPressed>,
    choices: Query<&Choice>,
    mut segments: Query<(&Choice, &mut Segment)>,
    recap: Option<ResMut<Recap>>,
    cycle: Option<Res<AutoCycle>>,
) {
    let Some(mut recap) = recap else {
        pressed.clear();
        return;
    };
    let digits = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
    ];
    let picked = pressed
        .read()
        .filter_map(|p| choices.get(p.0).ok().map(|c| c.0))
        .last()
        .or_else(|| {
            digits
                .iter()
                .zip(Metric::ALL)
                .find(|(k, _)| keys.just_pressed(**k))
                .map(|(_, metric)| metric)
        });
    let now = time.elapsed_secs();
    let picked = picked.or_else(|| {
        let every = cycle?.0;
        (now >= recap.since + recap.sweep + every).then(|| {
            let i = Metric::ALL
                .iter()
                .position(|m| *m == recap.metric)
                .unwrap_or(0);
            Metric::ALL[(i + 1) % Metric::ALL.len()]
        })
    });
    if let Some(metric) = picked
        && metric != recap.metric
    {
        recap.metric = metric;
        // Redraw, unless the first sweep is still going.
        if now > recap.since + recap.sweep {
            recap.since = now;
            recap.sweep = RESWEEP;
        }
    }
    for (choice, mut segment) in &mut segments {
        let lit = choice.0 == recap.metric;
        if segment.lit != lit {
            segment.lit = lit;
        }
    }
}

fn draw(
    mut lines: Gizmos<RecapLines>,
    mut glow: Gizmos<RecapGlow>,
    time: Res<Time>,
    cursor: Res<Cursor>,
    m: Res<Match>,
    recap: Res<Recap>,
) {
    let now = time.elapsed_secs();
    let shown = recap.shown(now);
    if shown <= 0.0 {
        return;
    }
    let history = &m.history;
    let end = history.end().max(1) as f32;
    let top = nice_top(history.peak(recap.metric));
    let grid = palette::FAINT.with_alpha(0.25 * shown);

    // The grid: quarters of the top value, the start and end, and the minutes.
    for q in 0..=4 {
        let r = BASE + (RIM - BASE) * q as f32 / 4.0;
        let color = if q == 0 {
            palette::RING.with_alpha(0.5 * shown)
        } else {
            grid
        };
        lines.linestrip_2d(ringmesh::arc(r, angle_at(1.0), TAU - GAP), color);
    }
    for f in [0.0, 1.0] {
        let dir = Vec2::from_angle(angle_at(f));
        lines.line_2d(dir * BASE, dir * RIM, grid);
    }
    let minutes = history.end() / TICK_HZ / 60;
    for k in 1..=minutes {
        let dir = Vec2::from_angle(angle_at((k * 60 * TICK_HZ) as f32 / end));
        let len = if k % 5 == 0 { 7.0 } else { 3.5 };
        lines.line_2d(dir * RIM, dir * (RIM + len), grid);
    }

    // The curves, drawing themselves clockwise.
    let drawn = recap.drawn(now);
    let seats = m.players.len();
    let mut curves = Vec::with_capacity(seats);
    for seat in 0..seats {
        let color = palette::seat(seat);
        let values = curve(history, seat, recap.metric);
        let mut line: Vec<Vec2> = Vec::new();
        for (i, &(f, v)) in values.iter().enumerate() {
            if f <= drawn {
                line.push(point(f, v, top));
                continue;
            }
            // Part of the way to the next sample.
            if let Some(&(fa, va)) = i.checked_sub(1).map(|j| &values[j]) {
                let t = ((drawn - fa) / (f - fa)).clamp(0.0, 1.0);
                line.push(point(drawn, va + (v - va) * t, top));
            }
            break;
        }
        curves.push(values);
        if line.len() < 2 {
            continue;
        }
        glow.linestrip_2d(line.iter().copied(), color.with_alpha(0.22));
        lines.linestrip_2d(line.iter().copied(), color);
        let head = *line.last().expect("two points");
        if drawn < 1.0 {
            glow.circle_2d(
                Isometry2d::from_translation(head),
                3.0,
                color.mix(&Color::WHITE, 0.5),
            )
            .resolution(10);
        } else if fell(history, seat).is_some() {
            // Out: a cross where the curve ends.
            let (d1, d2) = (Vec2::new(4.0, 4.0), Vec2::new(4.0, -4.0));
            lines.line_2d(head - d1, head + d1, color);
            lines.line_2d(head - d2, head + d2, color);
        }
    }

    // The moment under the pointer.
    if drawn >= 1.0
        && let Some(i) = hovered(&cursor, history)
    {
        let sample = &history.samples[i];
        let f = sample.tick as f32 / end;
        let dir = Vec2::from_angle(angle_at(f));
        lines.line_2d(dir * BASE, dir * RIM, palette::SELECTED.with_alpha(0.3));
        // On each curve as drawn; a curve that ended before has no dot.
        for (seat, values) in curves.iter().enumerate() {
            let Some(&(f, v)) = values.get(i) else {
                continue;
            };
            let at = point(f, v, top);
            let color = palette::seat(seat);
            lines
                .circle_2d(Isometry2d::from_translation(at), 3.0, color)
                .resolution(10);
            glow.circle_2d(
                Isometry2d::from_translation(at),
                3.0,
                color.with_alpha(0.35),
            )
            .resolution(10);
        }
    }
}

/// The disc behind the chart darkens as the chart fades in.
fn darken(
    time: Res<Time>,
    recap: Res<Recap>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    backdrop: Query<&Backdrop>,
) {
    let shown = recap.shown(time.elapsed_secs());
    for b in &backdrop {
        palette::tint(&mut materials, &b.0, Color::BLACK.with_alpha(0.45 * shown));
    }
}

/// The words in the middle: what is shown, when, and everyone's number.
#[allow(clippy::type_complexity)]
fn legend(
    time: Res<Time>,
    cursor: Res<Cursor>,
    m: Res<Match>,
    recap: Res<Recap>,
    mut texts: Query<(&Line, &mut Text2d, &mut TextColor, &mut Transform)>,
) {
    let history = &m.history;
    let metric = recap.metric;
    let shown = recap.shown(time.elapsed_secs());
    let at = (recap.drawn(time.elapsed_secs()) >= 1.0)
        .then(|| hovered(&cursor, history))
        .flatten();
    let sample = at.map_or_else(|| history.samples.last(), |i| history.samples.get(i));
    let Some(sample) = sample else {
        return;
    };
    let top = nice_top(history.peak(metric));

    // Everyone at that moment, best first; the fallen after the rest.
    let mut standings: Vec<(usize, u32, bool)> = (0..m.players.len())
        .map(|seat| {
            let out = fell(history, seat).is_some_and(|t| t <= sample.tick);
            (seat, sample.players[seat].get(metric), out)
        })
        .collect();
    standings.sort_by_key(|&(seat, v, out)| (out, std::cmp::Reverse(v), seat));

    for (line, mut text, mut color, mut transform) in &mut texts {
        let (content, ink) = match *line {
            Line::Heading => (metric.name().to_string(), palette::TEXT),
            Line::Caption => (metric.caption().to_string(), palette::DIM_TEXT),
            Line::When => {
                let when = if at.is_some() {
                    format!("at {}", clock(sample.tick))
                } else {
                    format!("at the end, {}", clock(sample.tick))
                };
                (when, palette::DIM_TEXT)
            }
            Line::Row(i) => match standings.get(i) {
                Some(&(seat, v, out)) => {
                    let name: String = m.players[seat].name.chars().take(11).collect();
                    let mark = if Some(seat as Seat) == m.me { '*' } else { ' ' };
                    let ink = palette::seat(seat).with_alpha(if out { 0.45 } else { 1.0 });
                    (format!("{name:<11}{mark}{:>6}", metric.format(v)), ink)
                }
                None => (String::new(), palette::TEXT),
            },
            Line::Scale(q) => {
                let v = top * q as u32 / 4;
                let r = BASE + (RIM - BASE) * q as f32 / 4.0;
                let spot = Vec2::new(0.0, r).extend(1.0);
                if transform.translation != spot {
                    transform.translation = spot;
                }
                (metric.format(v), palette::DIM_TEXT)
            }
            Line::Time => (text.0.clone(), palette::DIM_TEXT),
        };
        if text.0 != content {
            text.0 = content;
        }
        color.set_if_neq(TextColor(ink.with_alpha(ink.alpha() * shown)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cake_core::Sim;

    fn a_minute_of_history() -> History {
        let mut sim = Sim::new(2);
        let mut history = History::new(&sim);
        for _ in 0..60 * TICK_HZ {
            sim.step(&[]);
            history.observe(&sim);
        }
        history
    }

    fn pointer(f: f32, r: f32) -> Cursor {
        Cursor {
            ui: Some(Vec2::from_angle(angle_at(f)) * r),
            ..default()
        }
    }

    #[test]
    fn the_pointer_reads_off_the_moment_under_it() {
        let history = a_minute_of_history();
        assert_eq!(history.samples.len(), 61);
        assert_eq!(hovered(&pointer(0.0, 200.0), &history), Some(0));
        assert_eq!(hovered(&pointer(0.5, 200.0), &history), Some(30));
        assert_eq!(hovered(&pointer(1.0, 200.0), &history), Some(60));
        // Not in the middle, beyond the rim, or in the gap at the top.
        assert_eq!(hovered(&pointer(0.5, 60.0), &history), None);
        assert_eq!(hovered(&pointer(0.5, 400.0), &history), None);
        let gap = Cursor {
            ui: Some(Vec2::Y * 200.0),
            ..default()
        };
        assert_eq!(hovered(&gap, &history), None);
    }

    #[test]
    fn smoothing_softens_spikes_but_keeps_the_ends() {
        let at = |vs: &[f32]| -> Vec<(f32, f32)> {
            vs.iter().enumerate().map(|(i, v)| (i as f32, *v)).collect()
        };
        let spike = smooth(&at(&[0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0]));
        assert_eq!(spike[3].1, 2.0);
        assert_eq!(spike[0].1, 0.0);
        // A fall to zero at the end still reaches zero.
        let fall = smooth(&at(&[5.0, 5.0, 5.0, 5.0, 5.0, 0.0]));
        assert_eq!(fall[0].1, 5.0);
        assert_eq!(fall[5].1, 0.0);
        assert!(fall[4].1 < 5.0);
    }

    #[test]
    fn the_top_value_is_round_in_quarters() {
        for (peak, top) in [
            (0, 4),
            (3, 4),
            (13, 16),
            (37, 40),
            (41, 60),
            (620, 800),
            (1000, 1000),
        ] {
            assert_eq!(nice_top(peak), top, "peak {peak}");
            assert_eq!(nice_top(peak) % 4, 0);
        }
    }

    #[test]
    fn time_runs_clockwise_from_the_top_and_back() {
        for f in [0.0, 0.25, 0.5, 0.99, 1.0] {
            let back = share_at(angle_at(f)).expect("on the chart");
            assert!((back - f).abs() < 1e-4, "{f} came back as {back}");
        }
        // The gap at the top is not on the chart.
        assert!(share_at(FRAC_PI_2).is_none());
        // A quarter of the way through is at three o'clock, give or take the gap.
        assert!(angle_at(0.25).abs() < 0.1);
    }
}
