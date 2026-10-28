//! Hotkey name parsing (pure; shared by the engine-facing plugins).

/// Default idle-worker hotkey: F9 (VK 0x78) — unbound in stock Brood War and in the
/// mod's hotkey layer (F1–F8 are camera screens, F10 the menu, F11 the HUD toggle).
pub const DEFAULT_IDLE_WORKER_KEY: &str = "F9";

/// Default larva-pick hotkey: Ctrl+Space (VK 0x20 with the CTRL modifier) —
/// free in the game and the mod (the game binds nothing to Space alone, and no
/// mod hotkey uses Ctrl+Space; camera pan uses Ctrl+WASD). Flow: hold
/// Ctrl+Space, release Ctrl, type a count (2, 4, Enter = 24 larva). The
/// key-down is swallowed so nothing else fires. Players who prefer the old
/// single-key picker can set `CM_PLUS_LARVA_KEY=CAPSLOCK` (or any parseable
/// key/chord) in `cosmo_plus.conf`.
pub const DEFAULT_LARVA_KEY: &str = "CTRL+SPACE";

/// Modifier bits for a hotkey chord. Extra held modifiers are ignored: a
/// binding with these modifiers fires when AT LEAST these modifiers are held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Modifiers {
    pub const fn none() -> Self {
        Self { ctrl: false, shift: false, alt: false }
    }
    pub const fn any(&self) -> bool {
        self.ctrl || self.shift || self.alt
    }
    /// The virtual-key bits to test with GetAsyncKeyState.
    pub const fn mask(&self) -> u32 {
        (self.ctrl as u32) << 2 | (self.shift as u32) << 1 | (self.alt as u32)
    }
}

/// A parsed hotkey binding: the virtual key plus required modifiers (all
/// false for a bare key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub vk: u16,
    pub mods: Modifiers,
}

/// Parse a hotkey name into a virtual-key code.
///
/// Accepts `F1`–`F12`, single letters/digits, the punctuation keys (either the
/// character itself, e.g. `,` or `.`, or its name, e.g. `COMMA`), `SPACE`/`TAB`/
/// `HOME`/`END`/`DELETE`/`INSERT`/`PAGEUP`/`PAGEDOWN`, or a raw virtual-key code
/// in hex (`0x7A`).
pub fn parse_key(name: &str) -> Option<u16> {
    let n = name.trim();
    if n.is_empty() {
        return None;
    }
    let upper = n.to_ascii_uppercase();
    // F1..F12 = 0x70..0x7B.
    if let Some(rest) = upper.strip_prefix('F') {
        if let Ok(k) = rest.parse::<u32>() {
            if (1..=12).contains(&k) {
                return Some(0x6F + k as u16);
            }
        }
        return None;
    }
    let b = upper.as_bytes();
    // Single letter or digit.
    if b.len() == 1 && (b[0].is_ascii_alphabetic() || b[0].is_ascii_digit()) {
        return Some(b[0] as u16);
    }
    // Raw virtual-key code as hex (e.g. 0x7A).
    if let Some(hex) = upper.strip_prefix("0X") {
        if let Ok(v) = u16::from_str_radix(hex, 16) {
            return Some(v);
        }
    }
    // Punctuation: the character itself (`CM_PLUS_IDLE_WORKER_KEY=,`) or its name.
    if b.len() == 1 {
        if let Some(vk) = oem_key(b[0]) {
            return Some(vk);
        }
    }
    Some(match upper.as_str() {
        "SPACE" => 0x20,
        "TAB" => 0x09,
        "CAPSLOCK" | "CAPS" | "CAPITAL" => 0x14,
        "HOME" => 0x24,
        "END" => 0x23,
        "DELETE" => 0x2E,
        "INSERT" => 0x2D,
        "PAGEUP" => 0x21,
        "PAGEDOWN" => 0x22,
        "PAUSE" | "BREAK" => 0x13,
        "SCROLLLOCK" | "SCROLL" => 0x91,
        "COMMA" => 0xBC,
        "PERIOD" | "DOT" => 0xBE,
        "SLASH" => 0xBF,
        "SEMICOLON" => 0xBA,
        "APOSTROPHE" | "QUOTE" => 0xDE,
        "LBRACKET" | "LEFTBRACKET" => 0xDB,
        "RBRACKET" | "RIGHTBRACKET" => 0xDD,
        "BACKSLASH" => 0xDC,
        "MINUS" => 0xBD,
        "EQUALS" | "PLUS" => 0xBB,
        "BACKTICK" | "GRAVE" | "TILDE" => 0xC0,
        _ => return None,
    })
}

/// Default subgroup-cycle hotkey: Tab (VK 0x09). Unbound everywhere in stock Brood
/// War and in the mod's hotkey layer (the engine's key handler ignores it — see
/// `tab_cycle.rs`), so it is dedicated to SC2-style subgroup cycling. Shift+Tab
/// cycles backward; `CM_PLUS_TAB_PREV_KEY` may assign a separate backward key.
pub const DEFAULT_TAB_KEY: &str = "TAB";

/// Parse a hotkey name into a binding (key + required modifiers).
///
/// Accepts everything [`parse_key`] does, optionally prefixed with
/// `CTRL+`, `SHIFT+`, `ALT+` (any combination, any order, case-insensitive),
/// e.g. `CTRL+SPACE`, `SHIFT+CTRL+COMMA`, `ALT+0x50`. A modifier prefix on an
/// unparseable key yields `None`. Bare keys keep their legacy behaviour
/// (no modifiers required).
pub fn parse_binding(name: &str) -> Option<Binding> {
    let n = name.trim();
    if n.is_empty() {
        return None;
    }
    let upper = n.to_ascii_uppercase();
    let mut mods = Modifiers::none();
    let parts: Vec<&str> = upper.split('+').map(str::trim).collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return None; // stray separators, e.g. "CTRL++SPACE"
    }
    // All but the last part must be modifier names.
    for part in &parts[..parts.len() - 1] {
        match *part {
            "CTRL" | "CONTROL" => mods.ctrl = true,
            "SHIFT" => mods.shift = true,
            "ALT" => mods.alt = true,
            _ => return None, // unknown prefix
        }
    }
    // The last part is the key itself.
    let vk = parse_key(*parts.last().unwrap())?;
    Some(Binding { vk, mods })
}

/// US-layout OEM virtual-key codes for the printable punctuation keys.
fn oem_key(c: u8) -> Option<u16> {
    Some(match c {
        b',' => 0xBC,
        b'.' => 0xBE,
        b'/' => 0xBF,
        b';' => 0xBA,
        b'\'' => 0xDE,
        b'[' => 0xDB,
        b']' => 0xDD,
        b'\\' => 0xDC,
        b'-' => 0xBD,
        b'=' => 0xBB,
        b'`' => 0xC0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_and_hex_keys() {
        assert_eq!(parse_key("F9"), Some(0x78));
        assert_eq!(parse_key("f1"), Some(0x70));
        assert_eq!(parse_key("F12"), Some(0x7B));
        assert_eq!(parse_key("A"), Some(0x41));
        assert_eq!(parse_key("z"), Some(0x5A));
        assert_eq!(parse_key("7"), Some(0x37));
        assert_eq!(parse_key("SPACE"), Some(0x20));
        assert_eq!(parse_key("CAPSLOCK"), Some(0x14));
        assert_eq!(parse_key("caps"), Some(0x14));
        assert_eq!(parse_key("Home"), Some(0x24));
        assert_eq!(parse_key("0x7A"), Some(0x7A));
        assert_eq!(parse_key("Pause"), Some(0x13));
        assert_eq!(parse_key("scrolllock"), Some(0x91));
        assert_eq!(parse_key(","), Some(0xBC));
        assert_eq!(parse_key("COMMA"), Some(0xBC));
        assert_eq!(parse_key("."), Some(0xBE));
        assert_eq!(parse_key("PERIOD"), Some(0xBE));
        assert_eq!(parse_key(";"), Some(0xBA));
        assert_eq!(parse_key("/"), Some(0xBF));
        assert_eq!(parse_key("["), Some(0xDB));
        assert_eq!(parse_key("]"), Some(0xDD));
        assert_eq!(parse_key("'"), Some(0xDE));
        assert_eq!(parse_key("-"), Some(0xBD));
        assert_eq!(parse_key("="), Some(0xBB));
        assert_eq!(parse_key("`"), Some(0xC0));
        assert_eq!(parse_key("\\"), Some(0xDC));
        assert_eq!(parse_key(""), None);
        assert_eq!(parse_key("F13"), None);
        assert_eq!(parse_key("CTRL"), None);
        assert_eq!(parse_key("nonsense"), None);
    }

    #[test]
    fn parses_chords() {
        let b = |s: &str| parse_binding(s);
        assert_eq!(b("CTRL+SPACE"), Some(Binding { vk: 0x20, mods: Modifiers { ctrl: true, shift: false, alt: false } }));
        assert_eq!(b("ctrl+space"), Some(Binding { vk: 0x20, mods: Modifiers { ctrl: true, shift: false, alt: false } }));
        assert_eq!(b("SHIFT+CTRL+COMMA"), Some(Binding { vk: 0xBC, mods: Modifiers { ctrl: true, shift: true, alt: false } }));
        assert_eq!(b("ALT+0x50"), Some(Binding { vk: 0x50, mods: Modifiers { ctrl: false, shift: false, alt: true } }));
        assert_eq!(b("CONTROL+SHIFT+ALT+F12"), Some(Binding { vk: 0x7B, mods: Modifiers { ctrl: true, shift: true, alt: true } }));
        // Bare keys parse as bindings with no modifiers (legacy behaviour).
        assert_eq!(b("CAPSLOCK"), Some(Binding { vk: 0x14, mods: Modifiers::none() }));
        assert_eq!(b("F9"), Some(Binding { vk: 0x78, mods: Modifiers::none() }));
        // Errors.
        assert_eq!(b(""), None);
        assert_eq!(b("CTRL+"), None);
        assert_eq!(b("CTRL++SPACE"), None);
        assert_eq!(b("CTRL"), None);
        assert_eq!(b("CTRL+NONSENSE"), None);
        assert_eq!(b("WIN+SPACE"), None);
    }
}
