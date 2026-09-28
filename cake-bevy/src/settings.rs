//! Player settings: for now, the window style.
//!
//! The game is a circle. By default (native) it runs in a frameless,
//! transparent window whose outer ring is the window chrome: see
//! [`crate::chrome`]. A normal decorated window is available as a setting;
//! the web build is always an ordinary page.
//!
//! Settings persist in `$XDG_CONFIG_HOME/cake/settings` (or `~/.config`, or
//! `%APPDATA%` on Windows) as `key = value` lines. `CAKE_WINDOW=circle` or
//! `CAKE_WINDOW=windowed` overrides the file for one run.

use bevy::prelude::*;
use bevy::window::CompositeAlphaMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WindowStyle {
    /// Frameless and transparent: only the circle shows, and its outer ring
    /// is the window chrome.
    #[default]
    Circle,
    /// An ordinary decorated window with an opaque background.
    Windowed,
}

impl WindowStyle {
    fn parse(s: &str) -> Option<WindowStyle> {
        match s.trim() {
            "circle" => Some(WindowStyle::Circle),
            "windowed" => Some(WindowStyle::Windowed),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            WindowStyle::Circle => "circle",
            WindowStyle::Windowed => "windowed",
        }
    }

    pub fn toggled(self) -> WindowStyle {
        match self {
            WindowStyle::Circle => WindowStyle::Windowed,
            WindowStyle::Windowed => WindowStyle::Circle,
        }
    }
}

#[derive(Resource, Clone, Debug, PartialEq, Eq, Default)]
pub struct Settings {
    pub window: WindowStyle,
}

impl Settings {
    /// Read the settings: the file, then the environment override. The web
    /// is always windowed.
    pub fn load() -> Settings {
        let mut settings = Settings::default();
        if !circle_supported() {
            settings.window = WindowStyle::Windowed;
            return settings;
        }
        if let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) {
            settings = Settings::parse(&text);
        }
        if let Some(style) = std::env::var("CAKE_WINDOW")
            .ok()
            .and_then(|v| WindowStyle::parse(&v))
        {
            settings.window = style;
        }
        settings
    }

    fn parse(text: &str) -> Settings {
        let mut settings = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim() == "window"
                && let Some(style) = WindowStyle::parse(value)
            {
                settings.window = style;
            }
        }
        settings
    }

    fn render(&self) -> String {
        format!("window = {}\n", self.window.as_str())
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
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("cake").join("settings"))
}

#[cfg(target_family = "wasm")]
fn path() -> Option<std::path::PathBuf> {
    None
}

/// Can this platform show the circle window at all? Not on the web, where
/// the game is a page.
pub fn circle_supported() -> bool {
    !cfg!(target_family = "wasm")
}

/// Is this a Wayland session? winit prefers Wayland when it is.
pub fn wayland() -> bool {
    circle_supported() && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Is the desktop KDE Plasma?
pub fn kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|d| d.split(':').any(|part| part.eq_ignore_ascii_case("KDE")))
}

/// Does always-on-top work here? winit cannot ask Wayland for it, but KWin
/// can be asked directly (see `chrome::pin`).
pub fn pin_supported() -> bool {
    circle_supported() && (!wayland() || kde())
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
        for style in [WindowStyle::Circle, WindowStyle::Windowed] {
            let s = Settings { window: style };
            assert_eq!(Settings::parse(&s.render()), s);
        }
    }

    #[test]
    fn unknown_lines_and_values_are_ignored() {
        let s = Settings::parse("# comment\nwindow = sideways\ncolour = red\n");
        assert_eq!(s, Settings::default());
        let s = Settings::parse("window=windowed");
        assert_eq!(s.window, WindowStyle::Windowed);
    }
}
