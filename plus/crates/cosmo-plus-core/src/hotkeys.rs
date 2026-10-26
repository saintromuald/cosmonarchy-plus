//! Hotkey name parsing (pure; shared by the engine-facing plugins).

/// Default idle-worker hotkey: F9 (VK 0x78) — unbound in stock Brood War and in the
/// mod's hotkey layer (F1–F8 are camera screens, F10 the menu, F11 the HUD toggle).
pub const DEFAULT_IDLE_WORKER_KEY: &str = "F9";

/// Default larva-pick hotkey: CapsLock (VK 0x14) — free in the game and the
/// mod, and next to the digits for quick counts (CapsLock, 3, 3, Enter = 33
/// larva). The key-down is swallowed, so the OS caps toggle never flips.
pub const DEFAULT_LARVA_KEY: &str = "CAPSLOCK";

/// Default subgroup-cycle hotkey: Tab (VK 0x09). Unbound everywhere in stock Brood
/// War and in the mod's hotkey layer (the engine's key handler ignores it — see
/// `tab_cycle.rs`), so it is dedicated to SC2-style subgroup cycling. Shift+Tab
/// cycles backward; `CM_PLUS_TAB_PREV_KEY` may assign a separate backward key.
pub const DEFAULT_TAB_KEY: &str = "TAB";

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
}
