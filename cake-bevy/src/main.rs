use bevy::prelude::*;
use bevy::window::WindowResolution;
use cake_bevy::settings::{self, Settings, WindowStyle};
use cake_bevy::{logic_plugin, view_plugin, web};

fn main() {
    // Right-click issues orders; keep the browser's menu off it. No-op native.
    web::prevent_context_menu();
    let settings = Settings::load();
    let circle = settings.window == WindowStyle::Circle;
    let window = Window {
        title: "cake".into(),
        // The circle is the whole window: square, and small enough for a
        // laptop screen. A normal window gets room for the HUD's width.
        resolution: if circle {
            WindowResolution::new(820, 820)
        } else {
            WindowResolution::new(1100, 900)
        },
        decorations: !circle,
        // Transparency is decided when the window is created, so native
        // windows always ask for it; an opaque background then looks normal
        // and the style can change at runtime.
        transparent: settings::circle_supported(),
        composite_alpha_mode: settings::composite_alpha_mode(),
        fit_canvas_to_parent: true,
        ..default()
    };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(window),
        ..default()
    }))
    .insert_resource(settings)
    .add_plugins((logic_plugin, view_plugin));
    if let Some(shots) = Shots::from_env() {
        app.insert_resource(shots).add_systems(Update, dev_screenshot);
    }
    app.run();
}

/// Development aid (native): `CAKE_SHOT=path.png` saves screenshots of the
/// window, alpha included, at `CAKE_SHOT_AT` seconds (default 5; several as
/// `1,1.5,2`, saved as `path-0.png`, `path-1.png`, ...). It shows what the
/// game hands the compositor, whichever window has focus.
#[derive(Resource)]
struct Shots {
    /// When to take each, in seconds, and where to save it.
    due: Vec<(f32, String)>,
}

impl Shots {
    fn from_env() -> Option<Shots> {
        let path = std::env::var("CAKE_SHOT").ok()?;
        let times: Vec<f32> = std::env::var("CAKE_SHOT_AT")
            .unwrap_or_else(|_| "5".into())
            .split(',')
            .filter_map(|t| t.trim().parse().ok())
            .collect();
        let numbered = times.len() > 1;
        let due = times
            .into_iter()
            .enumerate()
            .map(|(i, t)| {
                let file = match path.rsplit_once('.') {
                    Some((stem, ext)) if numbered => format!("{stem}-{i}.{ext}"),
                    _ if numbered => format!("{path}-{i}"),
                    _ => path.clone(),
                };
                (t, file)
            })
            .rev()
            .collect();
        Some(Shots { due })
    }
}

fn dev_screenshot(mut commands: Commands, time: Res<Time>, mut shots: ResMut<Shots>) {
    if shots.due.last().is_none_or(|(at, _)| time.elapsed_secs() < *at) {
        return;
    }
    let Some((_, path)) = shots.due.pop() else {
        return;
    };
    // Bevy's own `save_to_disk` drops alpha, which is the point here.
    commands
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(
            move |shot: On<bevy::render::view::screenshot::ScreenshotCaptured>| {
                match shot.image.clone().try_into_dynamic() {
                    Ok(image) => {
                        if let Err(error) = image.into_rgba8().save(&path) {
                            error!(%error, "could not save the screenshot");
                        }
                    }
                    Err(error) => error!(%error, "could not convert the screenshot"),
                }
            },
        );
}
