use bevy::prelude::*;
use cake_bevy::{logic_plugin, palette, view_plugin, web};

fn main() {
    // Right-click issues orders; keep the browser's menu off it. No-op native.
    web::prevent_context_menu();
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "cake".into(),
                resolution: (1100u32, 900u32).into(),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(palette::BACKGROUND))
        .add_plugins((logic_plugin, view_plugin))
        .run();
}
