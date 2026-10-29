//! Dev REPL: a polled command file so the running game can be asked questions
//! without restarting it — the thing that turns "draw the badge somewhere
//! else" from a rebuild+relaunch+menu-click cycle into one command.
//!
//! Gated by `CM_PLUS_REPL=1` (never active in normal play). Each ~0.5 s the
//! draw pass reads `cosmo_plus.cmd` from the game directory, executes every
//! line and truncates the file; replies go to `cosmo_plus.repl.log`.
//!
//! Commands:
//!   badge on|off|default   toggle the idle badge / clear the pin
//!   badge <x> <y>          pin the badge to a screen position
//!   icon <id> <x> <y> [w] [h]   draw a command icon (scaled) for 3 s
//!   text <x> <y> [c] <str>   draw text (color id, default 4 = white) for 3 s
//!   idle                   select the next idle worker (as if the key fired)
//!   state                  re-verify globals and rewrite the state report
//!   frame                  report frame/camera/minimap rect
//!   units                  report total + idle worker counts
//!   reload-config          re-read cosmo_plus.conf and re-arm config-driven features

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Frame of the last command-file poll.
static LAST_POLL: AtomicU32 = AtomicU32::new(0);
/// Screen pin for the idle badge (`badge <x> <y>`).
static BADGE_PIN: Mutex<Option<(i32, i32)>> = Mutex::new(None);
/// Idle badge visibility (`badge off`).
static BADGE_SHOWN: AtomicBool = AtomicBool::new(true);

/// A transient on-screen probe (icon or text), shown for 3 s.
struct Probe {
    kind: ProbeKind,
    x: i32,
    y: i32,
    until: u32,
    color: u8,
    text: [u8; 32],
    w: i32,
    h: i32,
}

enum ProbeKind {
    Text,
    Icon(u32),
}

static PROBE: Lazy<Mutex<Option<Probe>>> = Lazy::new(|| Mutex::new(None));

pub(crate) fn install() {
    if crate::config::env_flag("CM_PLUS_REPL") {
        ENABLED.store(true, Ordering::SeqCst);
        crate::log::log("[cosmo-plus] repl: enabled (polling cosmo_plus.cmd)");
    }
}

fn reply(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("cosmo_plus.repl.log")
    {
        let _ = writeln!(f, "[{}] {msg}", crate::frame::frame_count_safe());
    }
}

fn parse_i32(s: &str) -> Option<i32> {
    s.trim().parse::<i32>().ok()
}

unsafe fn exec(line: &str) {
    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    match cmd {
        "zoom" => {
            let (cx, cy) = crate::overlay::camera();
            reply(&format!(
                "zoom units={} crop=({},{},{},{}) screen_layer={} scale={:.2} camera=({cx},{cy})",
                crate::hud::zoom_units(),
                crate::hud::crop_left(),
                crate::hud::crop_top(),
                crate::hud::crop_w(),
                crate::hud::crop_h(),
                crate::hud::screen_layer_active(),
                crate::hud::screen_scale(),
            ));
        }
        "mem" => {
            // mem <hex-address> <count> — hex-dump live memory to the log.
            let addr = parts.next().and_then(|s| usize::from_str_radix(s.trim_start_matches("0x"), 16).ok());
            let count = parts.next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(16).min(256);
            match addr {
                Some(a) if crate::overlay::readable_ptr(a as *const u8, count) => {
                    let mut out = String::new();
                    for i in 0..count {
                        out.push_str(&format!("{:02X} ", std::ptr::read((a + i) as *const u8)));
                    }
                    reply(&format!("mem 0x{a:08X}: {out}"));
                }
                _ => reply("usage: mem <hex-addr> [count] (readable memory only)"),
            }
        }
        "badge" => match parts.next() {
            Some("on") => {
                BADGE_SHOWN.store(true, Ordering::SeqCst);
                reply("badge shown");
            }
            Some("off") => {
                BADGE_SHOWN.store(false, Ordering::SeqCst);
                reply("badge hidden");
            }
            Some("default") => {
                *BADGE_PIN.lock().unwrap() = None;
                reply("badge back to the minimap anchor");
            }
            Some(x) => {
                let y = parts.next().and_then(parse_i32);
                match (parse_i32(x), y) {
                    (Some(x), Some(y)) => {
                        *BADGE_PIN.lock().unwrap() = Some((x, y));
                        reply(&format!("badge pinned to ({x},{y})"));
                    }
                    _ => reply("usage: badge on|off|default|<x> <y>"),
                }
            }
            None => reply("usage: badge on|off|default|<x> <y>"),
        },
        "icon" => {
            let (Some(id), Some(x), Some(y)) = (
                parts.next().and_then(|v| v.parse::<u32>().ok()),
                parts.next().and_then(parse_i32),
                parts.next().and_then(parse_i32),
            ) else {
                reply("usage: icon <id> <x> <y> [w] [h]");
                return;
            };
            let w = parts.next().and_then(parse_i32).unwrap_or(24);
            let h = parts.next().and_then(parse_i32).unwrap_or(22);
            let until = crate::frame::frame_count_safe() + 180;
            *PROBE.lock().unwrap() = Some(Probe {
                kind: ProbeKind::Icon(id),
                x,
                y,
                until,
                color: 0,
                text: [0; 32],
                w,
                h,
            });
            reply(&format!("icon {id} at ({x},{y}) {w}x{h} for 3 s"));
        }
        "text" => {
            let (Some(x), Some(y)) = (
                parts.next().and_then(parse_i32),
                parts.next().and_then(parse_i32),
            ) else {
                reply("usage: text <x> <y> [color] <string>");
                return;
            };
            let color = parts.next().and_then(|v| v.parse::<u8>().ok()).unwrap_or(4);
            let text = parts.collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                reply("usage: text <x> <y> [color] <string>");
                return;
            }
            let mut buf = [0u8; 32];
            let len = text.len().min(31);
            buf[..len].copy_from_slice(&text.as_bytes()[..len]);
            let until = crate::frame::frame_count_safe() + 180;
            *PROBE.lock().unwrap() = Some(Probe {
                kind: ProbeKind::Text,
                x,
                y,
                until,
                color,
                text: buf,
                w: 0,
                h: 0,
            });
            reply(&format!("text at ({x},{y}) color {color} for 3 s"));
        }
        "idle" => {
            crate::idle_workers::queue_press();
            reply("idle press queued");
        }
        "state" => {
            crate::state::write_report("repl");
            reply("state report rewritten");
        }
        "frame" => {
            let rect = crate::state::minimap_rect().unwrap_or([0; 4]);
            let (cx, cy) = crate::overlay::camera();
            reply(&format!(
                "frame={} camera=({cx},{cy}) minimap={rect:?}",
                crate::frame::frame_count_safe()
            ));
        }
        "units" => {
            let total = crate::frame::unit_count_safe();
            let idle = crate::idle_workers::count_idle(local_player_or_0());
            let mut per_player = [0usize; 12];
            crate::frame::for_each_unit(|u| {
                let p = (*u).player as usize;
                if p < 12 {
                    per_player[p] += 1;
                }
            });
            reply(&format!(
                "units={total} idle={idle} per_player={per_player:?}"
            ));
        }
        "reload-config" => {
            let applied = crate::config::load();
            crate::reload::rearm();
            reply(&format!("config reloaded: {applied} setting(s), features re-armed"));
        }
        "" => {}
        other => reply(&format!("unknown command {other:?}")),
    }
}

unsafe fn local_player_or_0() -> u32 {
    crate::idle_workers::local_player().unwrap_or(0)
}

/// Called from the draw pass; polls the command file at ~2 Hz.
pub(crate) unsafe fn poll() {
    if !ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let frame = crate::frame::frame_count_safe();
    let last = LAST_POLL.load(Ordering::SeqCst);
    if frame.wrapping_sub(last) < 24 {
        return;
    }
    LAST_POLL.store(frame, Ordering::SeqCst);
    let Ok(text) = std::fs::read_to_string("cosmo_plus.cmd") else {
        return;
    };
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let _ = std::fs::write("cosmo_plus.cmd", "");
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        exec(line);
    }
}

/// Draw the transient probes (called from the draw hook after the badge).
pub(crate) unsafe fn draw() {
    let Some(p) = PROBE.lock().unwrap().take() else {
        return;
    };
    if crate::frame::frame_count_safe() > p.until {
        return;
    }
    let font = crate::overlay::in_game_font();
    if font.is_null() {
        return;
    }
    crate::overlay::set_font_handle(font);
    crate::overlay::write_draw_text(0, 0, 0x7FFF, 0x7FFF);
    *(crate::overlay::G_PRINT_XY_ALIGNMENT as *mut u8) = 0x11;
    match p.kind {
        ProbeKind::Text => {
            crate::overlay::get_color_handle(p.color);
            crate::overlay::bw_print_xy(p.text.as_ptr(), p.x, p.y);
        }
        ProbeKind::Icon(id) => {
            let _ = crate::icon::draw_unit_icon_scaled(id, p.x, p.y, p.w, p.h);
        }
    }
    crate::overlay::set_font_handle(core::ptr::null_mut());
    // Re-arm for the next frame until it expires.
    *PROBE.lock().unwrap() = Some(p);
}

/// Badge visibility override (`badge off`).
pub(crate) fn badge_shown() -> bool {
    BADGE_SHOWN.load(Ordering::SeqCst)
}

/// Badge position pin (`badge <x> <y>`), or None to use the minimap anchor.
pub(crate) fn badge_pin() -> Option<(i32, i32)> {
    *BADGE_PIN.lock().unwrap()
}
