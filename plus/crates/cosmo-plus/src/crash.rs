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
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

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
    let mut buf = [0u8; 512];
    let mut s = Buf::new(&mut buf);
    let _ = write!(
        s,
        "[cosmo-crash] {}\n",
        std::env!("COSMO_PLUS_GIT_REV")
    );
    if !info.is_null() {
        let exc = &*(*info).ExceptionRecord;
        let ctx = &*(*info).ContextRecord;
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
    }
    let _ = write!(
        s,
        "  tick={} last_resolve units={:#x} flingy={:#x}\n",
        TICK.load(Ordering::Relaxed),
        LAST_RESOLVED_UNITS.load(Ordering::Relaxed),
        LAST_RESOLVED_FLINGY.load(Ordering::Relaxed),
    );
    #[cfg(target_arch = "x86")]
    {
        use windows_sys::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION};
        let mut mbi: MEMORY_BASIC_INFORMATION = core::mem::zeroed();
        let self_addr = crash_handler as usize as *const core::ffi::c_void;
        if VirtualQuery(
            self_addr as *const core::ffi::c_void,
            &mut mbi,
            core::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        ) != 0
        {
            let base = mbi.AllocationBase as usize;
            let _ = write!(s, "  image_base={:#x}\n", base);
            for (label, rva) in [
                ("0x5187e8", 0x5187e8usize),
                ("0x10074808", 0x10074808usize),
                ("0x1007480c", 0x1007480cusize),
                ("0x100752a2", 0x100752a2usize),
            ] {
                let v = *(base.wrapping_add(rva) as *const u32);
                let _ = write!(s, "  [{label}] = {v:#010x}\n");
            }
        }
    }
    let len = s.pos();
    drop(s);
    write_log(&buf, len);
    // Let StarCraft's own handler take over: EXCEPTION_CONTINUE_SEARCH.
    EXCEPTION_CONTINUE_SEARCH
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
