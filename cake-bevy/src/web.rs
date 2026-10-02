//! The room name and the random identities a session starts with.
//!
//! Adapted from chinese-checke.rs. On the web the room travels in the URL
//! fragment (`#room=name`), which makes a match shareable: send the link, and
//! the recipient lands in your lobby. Native builds take it from the
//! `CAKE_ROOM` environment variable instead. Either way, without one a fresh
//! room is generated.

use cake_net::RoomId;
#[cfg(not(target_family = "wasm"))]
use std::sync::atomic::{AtomicU64, Ordering};

/// The room named in the URL (web) or `CAKE_ROOM` (native), if valid. An
/// invalid name degrades to a fresh room rather than an error screen.
pub fn room_from_url() -> Option<RoomId> {
    #[cfg(target_family = "wasm")]
    {
        room_from_fragment(&read_fragment()?)
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let name = std::env::var("CAKE_ROOM").ok()?;
        RoomId::parse(&name).ok()
    }
}

/// Parse the room out of a URL fragment, given with or without its `#`.
#[cfg(any(target_family = "wasm", test))]
fn room_from_fragment(fragment: &str) -> Option<RoomId> {
    let fragment = fragment.strip_prefix('#').unwrap_or(fragment);
    for pair in fragment.split(['&', ';']) {
        if let Some(value) = pair.strip_prefix("room=") {
            return RoomId::parse(value).ok();
        }
    }
    None
}

/// Publish the room in the URL so the address bar is a share link, leaving
/// any other fragment params alone. No-op on native.
pub fn share_room(room: &RoomId) {
    #[cfg(target_family = "wasm")]
    write_fragment(&with_fragment_param(
        &read_fragment().unwrap_or_default(),
        "room",
        &room.0,
    ));
    #[cfg(not(target_family = "wasm"))]
    let _ = room;
}

/// `fragment` (with or without its `#`) with `key` set to `value`, every
/// other pair kept in place, so the room never clobbers state another part
/// of the app keeps in the fragment. Returned without the leading `#`.
#[cfg(any(target_family = "wasm", test))]
fn with_fragment_param(fragment: &str, key: &str, value: &str) -> String {
    let fragment = fragment.strip_prefix('#').unwrap_or(fragment);
    let mut pairs: Vec<String> = fragment
        .split(['&', ';'])
        .filter(|pair| !pair.is_empty() && pair.split('=').next() != Some(key))
        .map(str::to_string)
        .collect();
    pairs.push(format!("{key}={value}"));
    pairs.join("&")
}

/// Unambiguous lowercase, so a room read aloud survives the trip.
const ROOM_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// A freshly generated five-character room.
pub fn random_room() -> RoomId {
    let mut seed = fresh_seed();
    let mut id = String::new();
    for _ in 0..5 {
        let pick = (seed % ROOM_ALPHABET.len() as u64) as usize;
        seed /= ROOM_ALPHABET.len() as u64;
        id.push(ROOM_ALPHABET[pick] as char);
    }
    RoomId(id)
}

const PET_NAMES: &[&str] = &[
    "otter", "falcon", "maple", "ember", "comet", "panda", "lynx", "heron", "quail", "gecko",
    "koala", "raven", "tiger", "bison", "crane", "dingo", "eagle", "ibex", "orca", "yak", "hare",
    "moth", "wren", "toad", "newt", "elk", "fox", "owl", "bee", "finch", "mole", "starling",
    "puffin", "badger", "marten", "vole",
];

/// The name a session starts with: `CAKE_NAME` on native if set, otherwise a
/// random pet name.
pub fn player_name() -> String {
    #[cfg(not(target_family = "wasm"))]
    if let Ok(name) = std::env::var("CAKE_NAME")
        && !name.trim().is_empty()
    {
        return name.trim().chars().take(20).collect();
    }
    PET_NAMES[(fresh_seed() % PET_NAMES.len() as u64) as usize].to_string()
}

/// A seed that differs between processes and between calls. std has no
/// randomness on `wasm32-unknown-unknown`, so the web asks the browser.
#[cfg(target_family = "wasm")]
pub fn fresh_seed() -> u64 {
    (js_sys::Math::random() * (1u64 << 53) as f64) as u64
}

#[cfg(not(target_family = "wasm"))]
pub fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hash, Hasher};
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static START: std::sync::OnceLock<bevy::platform::time::Instant> = std::sync::OnceLock::new();
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    CALLS.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
    START
        .get_or_init(bevy::platform::time::Instant::now)
        .elapsed()
        .as_nanos()
        .hash(&mut hasher);
    hasher.finish()
}

/// Keep the browser's context menu off right-click, which issues orders.
/// No-op on native.
pub fn prevent_context_menu() {
    #[cfg(target_family = "wasm")]
    if let Some(window) = web_sys::window() {
        let callback = js_sys::Function::new_no_args("event.preventDefault();");
        let _ = window.add_event_listener_with_callback("contextmenu", &callback);
    }
}

#[cfg(target_family = "wasm")]
fn read_fragment() -> Option<String> {
    web_sys::window()?.location().hash().ok()
}

/// Replace the page's fragment in place: no navigation and no history
/// entry, so Back never lands on a bare page that redirects forward again.
#[cfg(target_family = "wasm")]
fn write_fragment(fragment: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let url = format!("#{fragment}");
    let written = match window.history() {
        Ok(history) => history
            .replace_state_with_url(&web_sys::wasm_bindgen::JsValue::NULL, "", Some(&url))
            .is_ok(),
        Err(_) => false,
    };
    if !written {
        let _ = window.location().set_hash(fragment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_room_is_read_from_the_fragment() {
        assert_eq!(room_from_fragment("#room=abc"), RoomId::parse("abc").ok());
        assert_eq!(
            room_from_fragment("x=1&room=q-7"),
            RoomId::parse("q-7").ok()
        );
        assert_eq!(room_from_fragment("#room=a/b"), None);
        assert_eq!(room_from_fragment(""), None);
    }

    #[test]
    fn sharing_a_room_keeps_other_fragment_params() {
        assert_eq!(with_fragment_param("#x=1", "room", "r1"), "x=1&room=r1");
        assert_eq!(
            with_fragment_param("#room=old&x=1", "room", "new"),
            "x=1&room=new"
        );
        assert_eq!(with_fragment_param("", "room", "r"), "room=r");
    }

    #[test]
    fn generated_rooms_are_valid() {
        for _ in 0..50 {
            let room = random_room();
            assert_eq!(RoomId::parse(&room.0), Ok(room));
        }
    }
}
