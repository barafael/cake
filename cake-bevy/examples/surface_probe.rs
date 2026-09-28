//! Print what this machine's window surface supports: formats, composite
//! alpha modes, present modes. For diagnosing window transparency.
//!
//! ```sh
//! cargo run -p cake-bevy --example surface_probe
//! ```

use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapter, RenderInstance};
use bevy::render::view::ExtractedWindows;
use bevy::render::{Render, RenderApp};
use bevy::window::CompositeAlphaMode;

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "surface probe".into(),
            resolution: (200u32, 200u32).into(),
            decorations: false,
            transparent: true,
            composite_alpha_mode: CompositeAlphaMode::PreMultiplied,
            ..default()
        }),
        ..default()
    }))
    .insert_resource(ClearColor(Color::NONE))
    .add_systems(Startup, |mut commands: Commands| {
        commands.spawn(Camera2d);
    })
    .add_systems(Update, (quit_after_a_moment, draw));
    app.sub_app_mut(RenderApp).add_systems(Render, probe);
    app.run();
}

fn probe(
    windows: Res<ExtractedWindows>,
    instance: Res<RenderInstance>,
    adapter: Res<RenderAdapter>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    for window in windows.windows.values() {
        *done = true;
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(window.handle.get_display_handle()),
            raw_window_handle: window.handle.get_window_handle(),
        };
        // SAFETY: the handles belong to a live window for the whole call.
        let surface = unsafe { instance.create_surface_unsafe(target) }.expect("a surface");
        let caps = surface.get_capabilities(&adapter);
        println!("adapter:       {:?}", adapter.get_info());
        println!("display:       {:?}", window.handle.get_display_handle());
        println!("formats:       {:?}", caps.formats);
        println!("alpha modes:   {:?}", caps.alpha_modes);
        println!("present modes: {:?}", caps.present_modes);
        println!(
            "configured:    format {:?}, alpha {:?}",
            window.swap_chain_texture_format, window.alpha_mode
        );
    }
}

/// Something opaque in the middle: everything around it should show the
/// desktop through.
fn draw(mut gizmos: Gizmos) {
    gizmos.circle_2d(Isometry2d::IDENTITY, 60.0, Color::WHITE);
}

/// `PROBE_SECS` keeps the window up longer, to look at it.
fn quit_after_a_moment(time: Res<Time>, mut exit: MessageWriter<AppExit>) {
    let hold = std::env::var("PROBE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.0);
    if time.elapsed_secs() > hold {
        exit.write(AppExit::Success);
    }
}
