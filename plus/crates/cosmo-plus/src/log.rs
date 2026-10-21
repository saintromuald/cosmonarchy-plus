//! Append-only log in the game's working directory (`cosmo_plus.log`).

use std::io::Write;

pub fn log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("cosmo_plus.log")
    {
        let _ = writeln!(f, "{}", msg);
    }
}
