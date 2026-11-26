//! Which code touches the title memory of textures the GPU writes
//! (`VICTORY_WATCH_GPU_TEXTURES=1`, diagnostics builds).
//!
//! On the console the CPU and the GPU share memory: what a shader renders
//! or stores into a texture is there for the CPU to read at the texture's
//! address. Here the contents of such a texture (a target, depth or
//! unordered-access texture, `resource::fills` false) exist on the host
//! card alone, and its title memory keeps whatever it held. A title that
//! reads such a texture on the CPU (game logic sampling a height or mask
//! the GPU made) reads stale bytes.
//!
//! Each such texture's whole 4 KiB pages are made inaccessible at every
//! presented frame ([`frame`]). The first access to one faults into our
//! vectored handler, which gives the pages their protection back, notes the
//! access (read or write, address, instruction) and lets it go on. At the
//! next frame the notes are logged, each instruction with the module it
//! lies in (the title's images are `victory-image-*.dll`; our own reads,
//! such as frame captures, are `victory.exe`), once per texture and
//! instruction, and the pages are guarded again.
//!
//! shortcut: other views of the same physical pages are not guarded, and a
//! system call reading a guarded page fails instead of faulting. Pages the
//! write watch (`watch.rs`) made read-only are guarded too; restoring the
//! protection a page had when it was guarded gives such a page back to it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

const PAGE: usize = 4096;

/// A guarded texture: its pages and what it is.
struct Range {
    end: usize,
    what: String,
    /// Inaccessible now, with the protection to give back.
    armed: Option<u32>,
}

/// An access the handler took, not logged yet.
struct Access {
    start: usize,
    write: bool,
    address: usize,
    rip: usize,
}

struct State {
    /// By first page address.
    ranges: BTreeMap<usize, Range>,
    accesses: Vec<Access>,
    /// (texture, instruction) pairs logged.
    logged: BTreeSet<(usize, usize)>,
    /// Accesses taken since the start, per module (diagnostics summary).
    by_module: BTreeMap<String, u64>,
}

static STATE: Mutex<State> = Mutex::new(State {
    ranges: BTreeMap::new(),
    accesses: Vec::new(),
    logged: BTreeSet::new(),
    by_module: BTreeMap::new(),
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether textures are watched (`VICTORY_WATCH_GPU_TEXTURES=1`).
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_WATCH_GPU_TEXTURES").is_some_and(|v| v == "1"))
}

/// The whole pages of `start..start + len`.
fn pages(start: usize, len: usize) -> Option<(usize, usize)> {
    let first = start.checked_add(PAGE - 1)? & !(PAGE - 1);
    let end = start.checked_add(len)? & !(PAGE - 1);
    (end > first).then_some((first, end))
}

/// Watches the GPU-written texture at `start` of `len` bytes, described by
/// `what`, from the next frame on.
pub(crate) fn watch(start: usize, len: usize, what: String) {
    if !on() {
        return;
    }
    let Some((first, end)) = pages(start, len) else {
        return;
    };
    install();
    let mut s = state();
    // A texture placed over an older one's pages takes them over.
    let older: Vec<usize> = s
        .ranges
        .range(..end)
        .filter(|(_, r)| r.end > first)
        .map(|(&k, _)| k)
        .collect();
    for k in older {
        if let Some(r) = s.ranges.remove(&k) {
            disarm(k, &r);
        }
    }
    s.ranges.insert(
        first,
        Range {
            end,
            what,
            armed: None,
        },
    );
}

/// Stops watching the texture at `start` of `len` bytes (it is released).
pub(crate) fn forget(start: usize, len: usize) {
    if !on() {
        return;
    }
    let Some((first, _)) = pages(start, len) else {
        return;
    };
    let mut s = state();
    if let Some(r) = s.ranges.remove(&first) {
        disarm(first, &r);
    }
}

fn disarm(first: usize, r: &Range) {
    if let Some(old) = r.armed {
        protect(first, r.end, old);
    }
}

/// Sets the protection of `first..end`; the one it had, if it was set.
fn protect(first: usize, end: usize, protection: u32) -> Option<u32> {
    use windows_sys::Win32::System::Memory::VirtualProtect;
    let mut old = 0;
    // SAFETY: title memory of a texture; only its protection changes.
    (unsafe { VirtualProtect(first as *const _, end - first, protection, &mut old) } != 0)
        .then_some(old)
}

/// Once per presented frame: logs the accesses taken since the last call
/// and guards every watched texture again.
pub(crate) fn frame(frame: u64) {
    use windows_sys::Win32::System::Memory::PAGE_NOACCESS;
    if !on() {
        return;
    }
    let mut s = state();
    let accesses = std::mem::take(&mut s.accesses);
    for a in accesses {
        let module = module_of(a.rip);
        *s.by_module.entry(module.0.clone()).or_default() += 1;
        if s.logged.len() < 2048 && s.logged.insert((a.start, a.rip)) {
            let what = s
                .ranges
                .get(&a.start)
                .map_or("released", |r| r.what.as_str())
                .to_owned();
            tracing::warn!(
                frame,
                kind = if a.write { "write" } else { "read" },
                address = format_args!("{:#x}", a.address),
                texture = format_args!("{:#x} {what}", a.start),
                module = module.0,
                offset = format_args!("{:#x}", module.1),
                "the CPU touches a texture the GPU writes"
            );
        }
    }
    if frame.is_multiple_of(256) && !s.by_module.is_empty() {
        tracing::info!(by_module = ?s.by_module, textures = s.ranges.len(), "GPU-written textures touched by the CPU so far");
    }
    for (&first, r) in s.ranges.iter_mut() {
        if r.armed.is_none() {
            r.armed = protect(first, r.end, PAGE_NOACCESS);
        }
    }
}

/// The file name of the module holding `at`, and `at`'s offset in it
/// ("?" and the address itself outside any module).
fn module_of(at: usize) -> (String, usize) {
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleFileNameW, GetModuleHandleExW,
    };
    let mut module = std::ptr::null_mut();
    // SAFETY: a query; the handle's count is not changed.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            at as *const u16,
            &mut module,
        )
    } != 0;
    if !found {
        return ("?".to_owned(), at);
    }
    let mut name = [0u16; 260];
    // SAFETY: a buffer of that many characters.
    let n = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
    let path = String::from_utf16_lossy(&name[..n.min(name.len())]);
    let file = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_owned();
    (file, at - module as usize)
}

/// Registers the handler, once, ahead of the others: it takes only faults
/// inside guarded textures.
fn install() {
    use windows_sys::Win32::Foundation::{EXCEPTION_ACCESS_VIOLATION, NTSTATUS};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH,
        EXCEPTION_POINTERS,
    };
    unsafe extern "system" fn on_access(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the system passes the exception being dispatched.
        let (r, context) = unsafe { (&*(*info).ExceptionRecord, &*(*info).ContextRecord) };
        if r.ExceptionCode != EXCEPTION_ACCESS_VIOLATION as NTSTATUS || r.NumberParameters < 2 {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let (kind, address) = (r.ExceptionInformation[0], r.ExceptionInformation[1]);
        let mut s = state();
        let Some((&start, range)) = s.ranges.range_mut(..=address).next_back() else {
            return EXCEPTION_CONTINUE_SEARCH;
        };
        if address >= range.end {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let Some(old) = range.armed.take() else {
            // Not ours to give back (opened meanwhile, or some other guard).
            return EXCEPTION_CONTINUE_SEARCH;
        };
        let end = range.end;
        if protect(start, end, old).is_none() {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        const WRITE: usize = 1;
        s.accesses.push(Access {
            start,
            write: kind == WRITE,
            address,
            rip: context.Rip as usize,
        });
        EXCEPTION_CONTINUE_EXECUTION
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: the handler only touches our own state and the protection
    // of the pages it guarded.
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(on_access));
    });
}

/// The accesses taken and not yet logged (tests).
#[cfg(test)]
fn pending() -> Vec<(usize, bool, usize)> {
    state()
        .accesses
        .iter()
        .map(|a| (a.start, a.write, a.address))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
    };

    #[test]
    fn a_read_and_a_write_of_a_guarded_texture_are_noted_and_go_on() {
        // SAFETY: setting the switch before anything reads it in this process.
        unsafe { std::env::set_var("VICTORY_WATCH_GPU_TEXTURES", "1") };
        assert!(on());
        const LEN: usize = 4 * PAGE;
        // SAFETY: fresh memory of our own.
        let base = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                LEN,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(base, 0);
        // SAFETY: our memory.
        unsafe { (base as *mut u32).add(1024).write(0x1234_5678) };
        watch(base, LEN, "test 64x64 f41".to_owned());
        frame(1);
        // A read from another thread: noted, the page given back, the value read.
        let at = base + PAGE;
        let got = std::thread::spawn(move || {
            // SAFETY: our memory; the guard is taken by the handler.
            unsafe { std::ptr::read_volatile(at as *const u32) }
        })
        .join()
        .unwrap();
        assert_eq!(got, 0x1234_5678);
        assert_eq!(pending(), vec![(base, false, at)]);
        // Logged and guarded again: a write is noted as one.
        frame(2);
        assert!(pending().is_empty());
        // SAFETY: as above.
        unsafe { std::ptr::write_volatile((base + 2 * PAGE) as *mut u32, 7) };
        assert_eq!(pending(), vec![(base, true, base + 2 * PAGE)]);
        // Released: no longer guarded, no longer noted.
        forget(base, LEN);
        frame(3);
        // SAFETY: as above.
        unsafe { std::ptr::read_volatile(base as *const u32) };
        assert!(pending().is_empty());
        // SAFETY: our own allocation.
        unsafe { VirtualFree(base as *mut _, 0, MEM_RELEASE) };
    }

    #[test]
    fn pages_are_the_whole_ones_inside() {
        assert_eq!(pages(0x1000, 0x3000), Some((0x1000, 0x4000)));
        assert_eq!(pages(0x1010, 0x3000), Some((0x2000, 0x4000)));
        assert_eq!(pages(0x1010, 0x100), None);
    }
}
