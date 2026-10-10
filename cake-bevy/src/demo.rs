//! Development aid (native): record a watched match to a video, hands off.
//!
//! `CAKE_RECORD=demo.mp4` (with `CAKE_BOTS`, `CAKE_WATCH=1` and, to shorten
//! it, `CAKE_SPEED`) records from the moment the match starts:
//!
//! - time advances exactly a video frame (1/30 s) per app frame, and every
//!   frame is captured and piped to ffmpeg, so the video plays at the true
//!   pace however slowly capturing runs;
//! - the pointer and the keyboard are ignored, so whoever is at the desk
//!   can't disturb it, and the window doesn't ask for focus;
//! - a director moves the camera: in close on the heaviest fighting, back out
//!   to the whole ring now and then;
//! - once the match is decided, the recap shows each of its metrics in turn,
//!   and then the app quits.
//!
//! `CAKE_RECORD_TITLE` is shown in the middle while it plays, and
//! `CAKE_RECORD_SECS` caps the video's length.

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use cake_core::Event;
use cake_core::geom::UNIT;

use crate::camera::{Cursor, Opening, Rig, track_cursor};
use crate::chrome::PointerSet;
use crate::hud::Caption;
use crate::recap::AutoCycle;
use crate::render::to_vec2;
use crate::{AppState, Match, fx};

pub const FPS: u32 = 30;
/// Seconds on each of the recap's metrics.
const RECAP_EACH: f32 = 3.0;
/// From the end of the match to the end of the video: the last explosions,
/// the recap's sweep, and every metric once.
const OUTRO: f32 = 1.2 + 1.6 + 5.0 * (RECAP_EACH + 0.6) + 1.0;

/// How close the director goes (the view's scale; 1 shows the whole ring).
const CLOSE_ZOOM: f32 = 0.4;
/// The title shows only while the view is at least this wide: in close, the
/// middle of the screen is where the fighting is.
const TITLE_ZOOM: f32 = 0.75;
/// Heat that makes a fight worth going in for, and below which it's over.
const ENTER_HEAT: f32 = 10.0;
const LEAVE_HEAT: f32 = 3.0;
/// How far around a spot its heat is gathered, in map units.
const GATHER: f32 = 90.0;
/// Heat fades with this time constant, in seconds.
const HEAT_FADE: f32 = 1.5;
/// Shortest wide shot, longest close shot, and how long a close shot must
/// run before it may jump to a hotter fight.
const MIN_WIDE: f32 = 3.0;
const MAX_CLOSE: f32 = 14.0;
const MIN_CLOSE: f32 = 6.0;
/// How quickly the camera eases toward where the director wants it, per
/// second.
const EASE: f32 = 1.6;

/// The recording: where it goes and how far it has got.
#[derive(Resource)]
pub struct Recording {
    path: String,
    /// The most frames to record.
    limit: Option<u64>,
    /// Frames asked for so far.
    requested: u64,
    /// When the match was decided, in app seconds.
    decided: Option<f32>,
    /// Set once no more frames are asked for; the frames still in flight are
    /// awaited, for at most this many more app frames.
    finishing: Option<u32>,
    sink: Arc<Mutex<Sink>>,
}

impl Recording {
    /// From the environment, if a recording was asked for.
    pub fn from_env() -> Option<Recording> {
        let path = std::env::var("CAKE_RECORD").ok()?;
        let limit = std::env::var("CAKE_RECORD_SECS")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .map(|s| (s * FPS as f32) as u64);
        Some(Recording {
            path,
            limit,
            requested: 0,
            decided: None,
            finishing: None,
            sink: Arc::new(Mutex::new(Sink::default())),
        })
    }
}

/// Captured frames on their way to ffmpeg, in order.
#[derive(Default)]
struct Sink {
    ffmpeg: Option<(Child, ChildStdin)>,
    /// The size of the first frame; frames of another size are dropped.
    size: Option<(u32, u32)>,
    /// The next frame to write, and frames that arrived ahead of it (`None`
    /// for one that was dropped).
    next: u64,
    early: BTreeMap<u64, Option<Vec<u8>>>,
    failed: bool,
}

impl Sink {
    fn take(&mut self, path: &str, index: u64, frame: Option<(u32, u32, Vec<u8>)>) {
        let frame = frame.and_then(|(w, h, rgba)| {
            let size = *self.size.get_or_insert((w, h));
            if size != (w, h) {
                warn!(w, h, "the window changed size; dropping the frame");
                return None;
            }
            Some(rgba)
        });
        self.early.insert(index, frame);
        while let Some(frame) = self.early.remove(&self.next) {
            self.next += 1;
            if let Some(rgba) = frame {
                self.write(path, &rgba);
            }
        }
    }

    fn write(&mut self, path: &str, rgba: &[u8]) {
        if self.failed {
            return;
        }
        if self.ffmpeg.is_none() {
            let (w, h) = self.size.expect("sized by the first frame");
            match spawn_ffmpeg(path, w, h) {
                Ok(pipe) => self.ffmpeg = Some(pipe),
                Err(error) => {
                    error!(%error, "could not start ffmpeg");
                    self.failed = true;
                    return;
                }
            }
        }
        if let Some((_, stdin)) = &mut self.ffmpeg
            && let Err(error) = stdin.write_all(rgba)
        {
            error!(%error, "ffmpeg stopped taking frames");
            self.failed = true;
        }
    }

    /// Close the pipe and wait for ffmpeg to finish the file.
    fn close(&mut self) {
        if let Some((mut child, stdin)) = self.ffmpeg.take() {
            drop(stdin);
            match child.wait() {
                Ok(status) if status.success() => {}
                Ok(status) => error!(%status, "ffmpeg failed"),
                Err(error) => error!(%error, "could not wait for ffmpeg"),
            }
        }
    }
}

fn spawn_ffmpeg(path: &str, w: u32, h: u32) -> std::io::Result<(Child, ChildStdin)> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
        ])
        .args(["-s", &format!("{w}x{h}"), "-r", &FPS.to_string(), "-i", "-"])
        // x264 wants even sides; transparent corners come out black.
        .args(["-vf", "crop=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p"])
        .args(["-c:v", "libx264", "-preset", "medium", "-crf", "18"])
        .args(["-movflags", "+faststart", path])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    let stdin = child.stdin.take().expect("piped");
    Ok((child, stdin))
}

/// Where the director has the camera.
#[derive(Resource, Debug)]
struct Director {
    shot: Shot,
    /// Recent fighting: where, how much, and when (app seconds).
    heat: Vec<(Vec2, f32, f32)>,
    /// Since when a close shot's fight has been too cool to stay for.
    cooling: Option<f32>,
    /// The demo's title, for wide shots.
    title: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shot {
    Wide { since: f32 },
    Close { at: Vec2, since: f32 },
}

pub fn plugin(app: &mut App) {
    let title = std::env::var("CAKE_RECORD_TITLE").ok();
    if let Some(title) = &title {
        app.insert_resource(Caption(title.clone()));
    }
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        1.0 / FPS as f64,
    )))
    .insert_resource(AutoCycle(RECAP_EACH))
    .insert_resource(Director {
        shot: Shot::Wide { since: 0.0 },
        heat: Vec::new(),
        cooling: None,
        title,
    })
    .add_systems(PreUpdate, hands_off.after(bevy::input::InputSystems))
    .add_systems(
        Update,
        (
            blind.in_set(PointerSet).after(track_cursor),
            direct
                .in_set(PointerSet)
                .after(blind)
                .before(fx::collect)
                .run_if(in_state(AppState::Game).and_then(resource_exists::<Match>)),
        ),
    )
    .add_systems(OnEnter(AppState::Game), |mut opening: ResMut<Opening>| {
        opening.restart();
    })
    .add_systems(Last, record.run_if(in_state(AppState::Game)));
}

/// Whatever the keys, buttons and wheel did this frame, nothing happened.
fn hands_off(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut scroll: ResMut<AccumulatedMouseScroll>,
    mut motion: ResMut<AccumulatedMouseMotion>,
) {
    keys.reset_all();
    buttons.reset_all();
    *scroll = AccumulatedMouseScroll::default();
    *motion = AccumulatedMouseMotion::default();
}

/// Nor is there a pointer: nothing lights up under it.
fn blind(mut cursor: ResMut<Cursor>) {
    *cursor = Cursor::default();
}

/// Point the camera: in on the heaviest fighting, out to the whole ring
/// between fights, after a while in close, and once it's all over.
fn direct(
    time: Res<Time>,
    m: Res<Match>,
    mut director: ResMut<Director>,
    mut rig: ResMut<Rig>,
    caption: Option<ResMut<Caption>>,
) {
    let now = time.elapsed_secs();
    let d = &mut *director;
    for event in &m.events {
        let (at, weight) = match *event {
            Event::Shot { to, .. } => (to, 0.3),
            Event::Impact { at, .. } => (at, 0.5),
            Event::Died { pos, kind, .. } if kind.is_structure() => (pos, 6.0),
            Event::Died { pos, .. } => (pos, 2.0),
            Event::Blast { at, radius, .. } => (at, (radius / UNIT) as f32 / 10.0),
            _ => continue,
        };
        d.heat.push((to_vec2(at), weight, now));
    }
    d.heat.retain(|(_, _, t)| now - t < 4.0 * HEAT_FADE);

    let heat_at = |spot: Vec2| -> (f32, Vec2) {
        let mut total = 0.0;
        let mut centre = Vec2::ZERO;
        for &(at, w, t) in &d.heat {
            if at.distance(spot) <= GATHER {
                let w = w * (-(now - t) / HEAT_FADE).exp();
                total += w;
                centre += at * w;
            }
        }
        (total, if total > 0.0 { centre / total } else { spot })
    };
    // The hottest fight: sampled at up to a few hundred recent spots.
    let step = (d.heat.len() / 300).max(1);
    let hottest = d
        .heat
        .iter()
        .step_by(step)
        .map(|&(at, _, _)| heat_at(at))
        .max_by(|a, b| a.0.total_cmp(&b.0));

    let over = m.sim.outcome.is_some();
    d.shot = match d.shot {
        _ if over => match d.shot {
            wide @ Shot::Wide { .. } => wide,
            Shot::Close { .. } => Shot::Wide { since: now },
        },
        Shot::Wide { since } => match hottest {
            Some((heat, at)) if heat >= ENTER_HEAT && now - since >= MIN_WIDE => {
                d.cooling = None;
                Shot::Close { at, since: now }
            }
            _ => d.shot,
        },
        Shot::Close { at, since } => {
            let (heat, centre) = heat_at(at);
            if heat < LEAVE_HEAT {
                d.cooling.get_or_insert(now);
            } else {
                d.cooling = None;
            }
            let cooled = d.cooling.is_some_and(|t| now - t >= 2.0);
            match hottest {
                _ if cooled || now - since >= MAX_CLOSE => Shot::Wide { since: now },
                // A much bigger fight elsewhere: go there.
                Some((hot, spot))
                    if now - since >= MIN_CLOSE
                        && hot > 2.0 * heat
                        && spot.distance(at) > GATHER =>
                {
                    Shot::Close {
                        at: spot,
                        since: now,
                    }
                }
                // Follow this one as it moves.
                _ => Shot::Close {
                    at: at.lerp(centre, 0.05),
                    since,
                },
            }
        }
    };

    let (pan, zoom) = match d.shot {
        Shot::Wide { .. } => (Vec2::ZERO, 1.0),
        Shot::Close { at, .. } => (at, CLOSE_ZOOM),
    };
    let k = 1.0 - (-EASE * time.delta_secs()).exp();
    rig.pan = rig.pan.lerp(pan, k);
    rig.zoom = (rig.zoom.ln() + (zoom.ln() - rig.zoom.ln()) * k).exp();

    if let (Some(mut caption), Some(title)) = (caption, &d.title) {
        let shown = if rig.zoom >= TITLE_ZOOM {
            title.as_str()
        } else {
            ""
        };
        if caption.0 != shown {
            caption.0 = shown.to_string();
        }
    }
}

/// Capture this frame, and end the recording when it's time.
fn record(
    mut commands: Commands,
    time: Res<Time>,
    m: Option<Res<Match>>,
    mut rec: ResMut<Recording>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    if rec.decided.is_none() && m.is_some_and(|m| m.sim.outcome.is_some()) {
        rec.decided = Some(now);
    }
    let done = rec.decided.is_some_and(|t| now - t >= OUTRO)
        || rec.limit.is_some_and(|limit| rec.requested >= limit);

    if let Some(left) = rec.finishing {
        let written = rec.sink.lock().map_or(0, |s| s.next);
        if written >= rec.requested || left == 0 {
            if let Ok(mut sink) = rec.sink.lock() {
                sink.close();
            }
            info!(frames = written, path = %rec.path, "recording finished");
            exit.write(AppExit::Success);
        } else {
            rec.finishing = Some(left - 1);
        }
        return;
    }
    if done {
        rec.finishing = Some(120);
        return;
    }

    let index = rec.requested;
    rec.requested += 1;
    let sink = rec.sink.clone();
    let path = rec.path.clone();
    commands
        .spawn(Screenshot::primary_window())
        .observe(move |shot: On<ScreenshotCaptured>| {
            let frame = shot.image.clone().try_into_dynamic().ok().map(|image| {
                let rgba = image.into_rgba8();
                (rgba.width(), rgba.height(), rgba.into_raw())
            });
            if let Ok(mut sink) = sink.lock() {
                sink.take(&path, index, frame);
            }
        });
}
