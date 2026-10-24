//! Crash self-capture: a first-chance vectored exception handler plus a panic
//! hook that write `cosmo_crash.log` next to the plugin, then hand control back
//! to the game's own handler (so Errors/*.ERR and _LastCrashReplay.rep are still
//! produced by StarCraft).
//!
//! The handler deliberately avoids allocation: it formats into a stack buffer
//! and writes through a handle opened once at install() time. Besides the
//! exception, it records the crash-time values of the bw_dat internals the
//! dat-read path lives on (dat globals), the last resolved dat pointers, and
//! the game tick counter.

use core::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::{HANDLE, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileA, WriteFile, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_ALWAYS,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
};

/// Game tick counter (incremented by the frame hook).
static TICK: AtomicU32 = AtomicU32::new(0);
/// Last dat table pointers resolved by data::init() (0 = never/None).
pub static LAST_RESOLVED_UNITS: AtomicUsize = AtomicUsize::new(0);
pub static LAST_RESOLVED_FLINGY: AtomicUsize = AtomicUsize::new(0);

/// Re-entrancy guard for the vectored handler (see `crash_handler`).
static IN_HANDLER: AtomicBool = AtomicBool::new(false);
/// `EXCEPTION_ACCESS_VIOLATION` (0xC0000005).
const ACCESS_VIOLATION: u32 = 0xC000_0005;

static mut LOG: HANDLE = INVALID_HANDLE_VALUE;

/// Increment the crash-context tick counter; call from the per-frame hook.
pub fn note_tick() {
    TICK.fetch_add(1, Ordering::Relaxed);
}

pub fn record_dat_resolve(units: usize, flingy: usize) {
    LAST_RESOLVED_UNITS.store(units, Ordering::Relaxed);
    LAST_RESOLVED_FLINGY.store(flingy, Ordering::Relaxed);
}

/// Minimal no-alloc writer into a fixed buffer.
struct Buf<'a> {
    b: &'a mut [u8],
    n: usize,
}

impl<'a> Buf<'a> {
    fn new(b: &'a mut [u8]) -> Self {
        Buf { b, n: 0 }
    }
    fn pos(&self) -> usize {
        self.n
    }
}

impl core::fmt::Write for Buf<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let rest = &mut self.b[self.n..];
        let take = s.len().min(rest.len());
        rest[..take].copy_from_slice(&s.as_bytes()[..take]);
        self.n += take;
        Ok(())
    }
}

unsafe fn write_log(buf: &[u8], len: usize) {
    if LOG == INVALID_HANDLE_VALUE || len == 0 {
        return;
    }
    let mut written = 0u32;
    WriteFile(
        LOG,
        buf.as_ptr(),
        len as u32,
        &mut written,
        core::ptr::null_mut(),
    );
}

unsafe extern "system" fn crash_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    // Re-entrancy guard: a fault *inside* this handler (as happened when the old
    // diagnostic reads poked past the module image) would re-dispatch to this same
    // handler and recurse until a stack overflow. Once we are in here, any nested
    // exception passes straight through to StarCraft's own handler.
    if IN_HANDLER.swap(true, Ordering::SeqCst) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let _ = write_av_report(info);
    IN_HANDLER.store(false, Ordering::SeqCst);
    // Let StarCraft's own handler take over: EXCEPTION_CONTINUE_SEARCH.
    EXCEPTION_CONTINUE_SEARCH
}

fn write_av_report(info: *mut EXCEPTION_POINTERS) {
    let mut buf = [0u8; 512];
    let mut s = Buf::new(&mut buf);
    let _ = write!(
        s,
        "[cosmo-crash] {}\n",
        std::env!("COSMO_PLUS_GIT_REV")
    );
    if info.is_null() {
        let len = s.pos();
        drop(s);
        unsafe { write_log(&buf, len) };
        return;
    }
    let exc = unsafe { &*(*info).ExceptionRecord };
    // Only access violations are ours to describe; everything else (debug prints,
    // breakpoints, C++ throws the engine handles) passes through untouched.
    if exc.ExceptionCode as u32 != ACCESS_VIOLATION {
        let len = s.pos();
        drop(s);
        unsafe { write_log(&buf, len) };
        return;
    }
    let ctx = unsafe { &*(*info).ContextRecord };
    let _ = write!(
        s,
        "  exception={:#010x} addr={:#010x}\n",
        exc.ExceptionCode as usize, exc.ExceptionAddress as usize
    );
    let _ = write!(
        s,
        "  eax={:#010x} ecx={:#010x} edx={:#010x} ebx={:#010x}\n",
        ctx.Eax, ctx.Ecx, ctx.Edx, ctx.Ebx
    );
    let _ = write!(
        s,
        "  esi={:#010x} edi={:#010x} esp={:#010x} ebp={:#010x} eip={:#010x}\n",
        ctx.Esi, ctx.Edi, ctx.Esp, ctx.Ebp, ctx.Eip
    );
    let _ = write!(
        s,
        "  tick={} last_resolve units={:#x} flingy={:#x}\n",
        TICK.load(Ordering::Relaxed),
        LAST_RESOLVED_UNITS.load(Ordering::Relaxed),
        LAST_RESOLVED_FLINGY.load(Ordering::Relaxed),
    );
    let len = s.pos();
    drop(s);
    unsafe { write_log(&buf, len) };
}

/// Install the crash handler + panic hook. Call once from plugin init.
pub unsafe fn install() {
    let path = b"cosmo_crash.log\0";
    let h = CreateFileA(
        path.as_ptr(),
        GENERIC_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        core::ptr::null_mut(),
        OPEN_ALWAYS,
        0,
        core::ptr::null_mut(),
    );
    if h != INVALID_HANDLE_VALUE {
        LOG = h;
    }
    AddVectoredExceptionHandler(1, Some(crash_handler));

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut buf = [0u8; 256];
        let mut s = Buf::new(&mut buf);
        let _ = write!(
            s,
            "[cosmo-crash] PANIC rev={}: {}\n",
            std::env!("COSMO_PLUS_GIT_REV"),
            info
        );
        let len = s.pos();
        drop(s);
        unsafe { write_log(&buf, len) };
        prev(info);
    }));
}
