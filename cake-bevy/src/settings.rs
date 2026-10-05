//! Player settings: for now, the display mode.
//!
//! The game is a circle. By default (native) it runs in **cake mode**: a
//! frameless, transparent window of which only the circle shows, whose outer
//! ring is the window chrome (see [`crate::chrome`]). **Window mode** is an
//! ordinary decorated window. The web build is always an ordinary page.
//!
//! Settings persist in `$XDG_CONFIG_HOME/cake/settings` (or `~/.config`, or
//! `%APPDATA%` on Windows) as `key = value` lines. `CAKE_MODE=cake` or
//! `CAKE_MODE=window` overrides the file for one run.

use bevy::prelude::*;
use bevy::window::CompositeAlphaMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DisplayMode {
    /// Cake mode: frameless and transparent, only the circle shows, and its
    /// outer ring is the window chrome.
    #[default]
    Cake,
    /// Window mode: an ordinary decorated window with an opaque background.
    Window,
}

impl DisplayMode {
    fn parse(s: &str) -> Option<DisplayMode> {
        match s.trim() {
            // The earlier names, as older settings files have them.
            "cake" | "circle" => Some(DisplayMode::Cake),
            "window" | "windowed" => Some(DisplayMode::Window),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            DisplayMode::Cake => "cake",
            DisplayMode::Window => "window",
        }
    }

    /// As players see it named.
    pub fn name(self) -> &'static str {
        match self {
            DisplayMode::Cake => "cake mode",
            DisplayMode::Window => "window mode",
        }
    }

    pub fn toggled(self) -> DisplayMode {
        match self {
            DisplayMode::Cake => DisplayMode::Window,
            DisplayMode::Window => DisplayMode::Cake,
        }
    }
}

#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub mode: DisplayMode,
    /// Shots, sparks, smoke, wakes and stains. Calm machines can do without.
    pub effects: bool,
    /// The procedural nebula behind the map. It costs a small re-render
    /// ten times a second, which the web feels.
    pub sky: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            mode: DisplayMode::default(),
            effects: true,
            sky: true,
        }
    }
}

/// `on`/`off`, and friends.
fn parse_bool(s: &str) -> Option<bool> {
    match s.trim() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

fn bool_str(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

impl Settings {
    /// Read the settings: the file, then the environment override. The web
    /// is always in window mode.
    pub fn load() -> Settings {
        let mut settings = Settings::default();
        if !cake_supported() {
            settings.mode = DisplayMode::Window;
            return settings;
        }
        if let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) {
            settings = Settings::parse(&text);
        }
        if let Some(mode) = std::env::var("CAKE_MODE")
            .ok()
            .and_then(|v| DisplayMode::parse(&v))
        {
            settings.mode = mode;
        }
        for (key, field) in [("CAKE_EFFECTS", 0usize), ("CAKE_SKY", 1)] {
            if let Some(v) = std::env::var(key).ok().and_then(|v| parse_bool(&v)) {
                if field == 0 {
                    settings.effects = v;
                } else {
                    settings.sky = v;
                }
            }
        }
        settings
    }

    fn parse(text: &str) -> Settings {
        let mut settings = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            // `window` is the key older settings files used.
            if matches!(key.trim(), "mode" | "window")
                && let Some(mode) = DisplayMode::parse(value)
            {
                settings.mode = mode;
            }
            match key.trim() {
                "effects" => {
                    if let Some(v) = parse_bool(value) {
                        settings.effects = v;
                    }
                }
                "sky" => {
                    if let Some(v) = parse_bool(value) {
                        settings.sky = v;
                    }
                }
                _ => {}
            }
        }
        settings
    }

    fn render(&self) -> String {
        format!(
            "mode = {}\neffects = {}\nsky = {}\n",
            self.mode.as_str(),
            bool_str(self.effects),
            bool_str(self.sky)
        )
    }

    /// Write the settings back. Failing to save is not worth stopping for.
    pub fn save(&self) {
        let Some(path) = path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(error) = std::fs::write(&path, self.render()) {
            warn!(%error, path = %path.display(), "could not save settings");
        }
    }
}

#[cfg(not(target_family = "wasm"))]
fn path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("cake").join("settings"))
}

#[cfg(target_family = "wasm")]
fn path() -> Option<std::path::PathBuf> {
    None
}

/// Can this platform show cake mode at all? Not on the web, where the game
/// is a page.
pub fn cake_supported() -> bool {
    !cfg!(target_family = "wasm")
}

/// Is this a Wayland session? winit prefers Wayland when it is.
pub fn wayland() -> bool {
    cake_supported() && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Is the desktop KDE Plasma?
pub fn kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|d| d.split(':').any(|part| part.eq_ignore_ascii_case("KDE")))
}

/// Does always-on-top work here? winit cannot ask Wayland for it, but KWin
/// can be asked directly (see `chrome::pin`).
pub fn pin_supported() -> bool {
    cake_supported() && (!wayland() || kde())
}

/// How the window's surface is composited. Transparency needs a
/// non-opaque mode, and it can only be chosen when the window is created, so
/// native windows always get one: an opaque background then just looks
/// normal, and switching style at runtime keeps working.
pub fn composite_alpha_mode() -> CompositeAlphaMode {
    if cfg!(target_os = "macos") {
        CompositeAlphaMode::PostMultiplied
    } else if cfg!(target_os = "linux") {
        CompositeAlphaMode::PreMultiplied
    } else {
        // Windows swapchains are opaque: the circle works, but its corners
        // show as black.
        CompositeAlphaMode::Auto
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_their_text() {
        for mode in [DisplayMode::Cake, DisplayMode::Window] {
            for effects in [true, false] {
                for sky in [true, false] {
                    let s = Settings {
                        mode,
                        effects,
                        sky,
                    };
                    assert_eq!(Settings::parse(&s.render()), s);
                }
            }
        }
    }

    #[test]
    fn unknown_lines_and_values_are_ignored() {
        let s = Settings::parse("# comment\nwindow = sideways\ncolour = red\n");
        assert_eq!(s, Settings::default());
        let s = Settings::parse("mode=window");
        assert_eq!(s.mode, DisplayMode::Window);
    }

    #[test]
    fn older_settings_files_still_read() {
        assert_eq!(
            Settings::parse("window = windowed").mode,
            DisplayMode::Window
        );
        assert_eq!(Settings::parse("window = circle").mode, DisplayMode::Cake);
    }
}
