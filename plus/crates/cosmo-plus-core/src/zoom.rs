//! Player zoom-out floor for the mod's `WorldZoom` system.
//!
//! The widescreen `gptp.qdp` implements zoom in `systems/display/zoom.*`
//! (reference: `cosmonarchy-gptp`). Each role gets its floor from
//! `apply_display_role()`:
//!
//! ```cpp
//! const bool full_fov = bw::in_replay_or_witness();
//! g_display.set_min_zoom_percent(full_fov ? zoom::WitnessMinPercent   // 100
//!                                         : zoom::PlayerMinPercent);  // 200
//! g_zoom.begin_game();
//! ```
//!
//! Witnesses/replays may wheel from the 200% default down to 100% — 8 notches
//! of 12.5%. Stock players are held at 200% (no zoom-out). Plus rewrites the
//! player branch's immediate to 125% — exactly 6 of the witness's 8 notches,
//! i.e. **3/4 of the observer-mode zoom depth** — with a single 5-byte write
//! at plugin load. The witness branch and everything else are untouched.
//!
//! Compiled site (widescreen release `gptp.qdp`, MSVC x86):
//!
//! ```asm
//! BA 64 00 00 00           mov   edx, 0x64      ; witness floor (100)
//! B8 C8 00 00 00           mov   eax, 0xc8      ; player floor (200)  <- patched
//! 0F 45 C2                 cmovne eax, edx
//! 50                       push  eax
//! E8 xx xx xx xx           call  set_min_zoom_percent
//! ```
//!
//! The call target must contain the clamp and world-extent math signatures
//! before the patch is accepted, so a stray pattern match can never be
//! written to.

/// Player floor Plus installs: 125% — 3/4 of the observer-mode zoom depth.
pub const PLAYER_FLOOR_PERCENT: u32 = 125;
/// Witness/replay floor; read-only here, for documentation and tests.
pub const WITNESS_FLOOR_PERCENT: u32 = 100;
/// Player floor compiled into the unpatched widescreen `gptp.qdp`.
pub const STOCK_PLAYER_FLOOR_PERCENT: u32 = 200;
/// Engine clamp bounds `set_min_zoom_percent` enforces: [100, 300].
pub const MIN_PERCENT: u32 = 100;
pub const MAX_PERCENT: u32 = 300;

/// Byte signature of the compiled `apply_display_role` player branch.
///
/// 15 bytes ending in the `call` opcode; the rel32 target follows.
pub const SITE_PATTERN: &[u8] =
    b"\xBA\x64\x00\x00\x00\xB8\xC8\x00\x00\x00\x0F\x45\xC2\x50\xE8";
/// Offset of the `mov eax, imm32` immediate inside [`SITE_PATTERN`].
pub const MOV_IMM_OFFSET: usize = 5;
/// Length of the `mov eax, imm32` instruction that is rewritten.
pub const MOV_IMM_LEN: usize = 5;

/// `cmp ecx, 0x64` — start of `set_min_zoom_percent`'s clamp to [100, 300].
const TARGET_CLAMP_SIG: &[u8] = b"\x83\xF9\x64";
/// `mov ebx, 0x2710` — the 10000-unit world-extent math in the same function.
const TARGET_WORLD_MATH_SIG: &[u8] = b"\xBB\x10\x27\x00\x00";

/// A located and verified patch site, image-relative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoomFloorSite {
    /// Image offset of the `mov eax, imm32` immediate to rewrite.
    pub mov_imm: usize,
    /// Image offset of the `set_min_zoom_percent` entry the call targets.
    pub call_target: usize,
}

/// Search `image` (a mapped module, or a faithful re-map of its file) for the
/// compiled `apply_display_role` site and verify its call target is
/// `set_min_zoom_percent`. Returns `None` when the build predates widescreen
/// or the layout has changed.
pub fn find_site(image: &[u8]) -> Option<ZoomFloorSite> {
    let pos = find_bytes(image, SITE_PATTERN)?;
    let mov_imm = pos + MOV_IMM_OFFSET;
    let rel_off = pos + SITE_PATTERN.len();
    let rel = i32::from_le_bytes(image.get(rel_off..rel_off + 4)?.try_into().ok()?);
    let call_target = (rel_off as i64 + 4 + rel as i64) as usize;
    let target = image.get(call_target..)?;
    if !window_contains(target, TARGET_CLAMP_SIG, 24)
        || !window_contains(target, TARGET_WORLD_MATH_SIG, 64)
    {
        return None;
    }
    Some(ZoomFloorSite { mov_imm, call_target })
}

/// The 5 bytes currently at the site, or `None` if out of bounds.
pub fn current_immediate(image: &[u8], site: ZoomFloorSite) -> Option<[u8; 5]> {
    image
        .get(site.mov_imm..site.mov_imm + MOV_IMM_LEN)?
        .try_into()
        .ok()
}

/// Encode `mov eax, percent` for the patch.
pub fn encode_mov_imm(percent: u32) -> [u8; 5] {
    let imm = percent.to_le_bytes();
    [0xB8, imm[0], imm[1], imm[2], imm[3]]
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

fn window_contains(bytes: &[u8], sig: &[u8], window: usize) -> bool {
    bytes
        .get(..window.min(bytes.len()))
        .is_some_and(|w| find_bytes(w, sig).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Re-map a PE file the way the loader does, so image offsets in tests
    /// match the ones the plugin sees in memory.
    fn map_image(file: &[u8]) -> Option<Vec<u8>> {
        if file.len() < 0x40 {
            return None;
        }
        let e_lfanew = u32::from_le_bytes(file[0x3c..0x40].try_into().ok()?) as usize;
        if &file[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
            return None;
        }
        let opt = e_lfanew + 24;
        let magic = u16::from_le_bytes(file[opt..opt + 2].try_into().ok()?);
        let (sec_off, num_secs, size_of_image) = if magic == 0x10b {
            // PE32
            (
                opt + 96 + u32::from_le_bytes(file[opt + 60..opt + 64].try_into().ok()?) as usize,
                u16::from_le_bytes(file[opt + 2..opt + 4].try_into().ok()?) as usize,
                u32::from_le_bytes(file[opt + 56..opt + 60].try_into().ok()?) as usize,
            )
        } else {
            // PE32+
            (
                opt + 112 + u32::from_le_bytes(file[opt + 60..opt + 64].try_into().ok()?) as usize,
                u16::from_le_bytes(file[opt + 2..opt + 4].try_into().ok()?) as usize,
                u32::from_le_bytes(file[opt + 56..opt + 60].try_into().ok()?) as usize,
            )
        };
        let mut image = vec![0u8; size_of_image];
        let hdr_len = (file.len()).min(image.len());
        image[..hdr_len].copy_from_slice(&file[..hdr_len]);
        for i in 0..num_secs {
            let sec = sec_off + i * 40;
            let vsize = u32::from_le_bytes(file[sec + 8..sec + 12].try_into().ok()?) as usize;
            let va = u32::from_le_bytes(file[sec + 12..sec + 16].try_into().ok()?) as usize;
            let raw = u32::from_le_bytes(file[sec + 20..sec + 24].try_into().ok()?) as usize;
            let rsize = u32::from_le_bytes(file[sec + 16..sec + 20].try_into().ok()?) as usize;
            let n = vsize.min(rsize);
            if raw + n <= file.len() && va + n <= image.len() {
                image[va..va + n].copy_from_slice(&file[raw..raw + n]);
            }
        }
        Some(image)
    }

    fn sample() -> Option<Vec<u8>> {
        let path = std::env::var("COSMO_GPTP_DLL").ok()?;
        let bytes = std::fs::read(&path).ok()?;
        map_image(&bytes)
    }

    #[test]
    fn finds_and_verifies_the_role_floor_site() {
        let Some(image) = sample() else {
            eprintln!("skipping: COSMO_GPTP_DLL not set");
            return;
        };
        let site = find_site(&image).expect("apply_display_role site not found");
        let bytes = current_immediate(&image, site).expect("immediate readable");
        assert_eq!(bytes, encode_mov_imm(STOCK_PLAYER_FLOOR_PERCENT));
        // The call target must look like set_min_zoom_percent: clamp to
        // [100, 300] followed by the 10000-unit world-extent math.
        let t = &image[site.call_target..site.call_target + 64];
        assert!(find_bytes(t, TARGET_CLAMP_SIG).is_some());
        assert!(find_bytes(t, TARGET_WORLD_MATH_SIG).is_some());
    }

    #[test]
    fn encodes_three_quarters_floor_on_the_ladder() {
        let b = encode_mov_imm(PLAYER_FLOOR_PERCENT);
        assert_eq!(b, [0xB8, 0x7D, 0, 0, 0]);
        // 125% = 6 notches of 12.5% below the 200% default; the witness gets 8.
        assert_eq!(
            (STOCK_PLAYER_FLOOR_PERCENT - PLAYER_FLOOR_PERCENT) * 8,
            (STOCK_PLAYER_FLOOR_PERCENT - WITNESS_FLOOR_PERCENT) * 6
        );
        assert!(PLAYER_FLOOR_PERCENT >= MIN_PERCENT && PLAYER_FLOOR_PERCENT < MAX_PERCENT);
    }

    #[test]
    fn signature_search_fails_cleanly_on_garbage() {
        assert!(find_site(&[0u8; 4096]).is_none());
        assert!(find_site(b"").is_none());
    }
}
