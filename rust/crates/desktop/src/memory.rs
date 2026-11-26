//! Where Sigil's memory goes, for Settings > About: the Rust heap (counted by
//! `procwatch::alloc::Counting`, the global allocator), the decoded images the app holds, and
//! the whole process as the OS counts it (Task Manager's "Memory", Activity Monitor's).
//! The difference between the process and the heap is the graphics driver, the window's
//! buffers and system libraries.

use crate::app::State;

/// GPU texture bytes for thumbnail-sized pictures (Rune's atlas shelf pages, images up to 1024
/// pixels a side, every window together): Browse cards at 2x are about 0.25 MB each on the GPU,
/// so 32 MB holds several screens of them. Pages not drawn in the last few frames are freed,
/// least recently drawn first, while over it.
pub const THUMB_BUDGET: u64 = 32 << 20;
/// GPU texture bytes for all pictures: the thumbnails plus large ones (the billboard, a
/// gallery or lightbox picture, the Crosshair backdrop, Locker renders).
pub const IMAGE_BUDGET: u64 = 64 << 20;
/// The largest picture decoded at all, in pixels (4096 by 4096, 64 MB of RGBA while it is
/// scaled): GameBanana originals reach 7960 by 4412. A larger one shows its placeholder.
pub const MAX_DECODE_PIXELS: u64 = 4096 * 4096;

/// Sets Rune's image budgets and decode limit (`app::setup`).
pub fn configure_images() {
    rune::set_image_budget(THUMB_BUDGET, IMAGE_BUDGET);
    rune::set_max_decode_pixels(MAX_DECODE_PIXELS);
}

pub struct Reading {
    /// Live bytes the Rust allocator handed out.
    pub heap: Option<u64>,
    /// The process's private memory (Windows commit, macOS physical footprint).
    pub process: Option<u64>,
    /// Windows: the private working set, the number Task Manager's Memory column shows.
    pub task_manager: Option<u64>,
    /// Decoded images held, and their estimated bytes with mips.
    pub images: (usize, u64),
}

pub fn read(st: &State) -> Reading {
    let heap = procwatch::alloc::heap().map(|h| h.live.max(0) as u64);
    let mut count = 0;
    let mut bytes = 0u64;
    for px in st.thumbnails.values() {
        let (w, h) = px.size();
        if w > 0 && h > 0 {
            count += 1;
            // RGBA with the mip chain (a third more).
            bytes += u64::from(w) * u64::from(h) * 4 * 4 / 3;
        }
    }
    let (process, task_manager) = process_bytes();
    Reading { heap, process, task_manager, images: (count, bytes) }
}

/// Logs Rune's GPU allocator report when a window took a new one (`poll`, debug level): at its
/// first picture upload, when it went idle after drawing (a scroll burst) and when it went out
/// of sight (minimised). D3D12 and Vulkan only; Metal has none.
pub fn log_gpu_allocator(last: &mut u64) {
    let Some(report) = rune::gpu_allocator_report() else { return };
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    report.hash(&mut hasher);
    let hash = hasher.finish();
    if hash != *last {
        *last = hash;
        crate::screens::settings::log::debug(report);
    }
}

/// Committed private memory and the private working set.
#[cfg(windows)]
fn process_bytes() -> (Option<u64>, Option<u64>) {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters: PROCESS_MEMORY_COUNTERS_EX2 = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
    counters.cb = size;
    // SAFETY: the pseudo handle needs no closing; `counters` is `size` bytes.
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>(),
            size,
        )
    };
    if ok == 0 {
        return (None, None);
    }
    (Some(counters.PrivateUsage as u64), Some(counters.PrivateWorkingSetSize as u64))
}

#[cfg(target_os = "macos")]
fn process_bytes() -> (Option<u64>, Option<u64>) {
    (procwatch::usage().footprint, None)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn process_bytes() -> (Option<u64>, Option<u64>) {
    (None, None)
}

/// A plain-text report for a bug report: the reading, and on Windows where the committed memory
/// sits (private allocations by size, mapped sections, and each DLL's writable pages), from a walk
/// of the address space with `VirtualQuery`. No paths beyond DLL names.
pub fn report(st: &State) -> String {
    let m = read(st);
    let opt = |b: Option<u64>| b.map_or_else(|| "-".to_string(), mb);
    let mut out = format!(
        "Sigil {} memory on {}\nTask Manager (private working set): {}\nCommitted private: {}\nApp heap: {}\nImages: {} in {}\n",
        crate::updater::VERSION,
        std::env::consts::OS,
        opt(m.task_manager),
        opt(m.process),
        opt(m.heap),
        mb(m.images.1),
        m.images.0,
    );
    out.push_str(&regions());
    out
}

#[cfg(windows)]
fn regions() -> String {
    use std::collections::HashMap;
    use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_IMAGE, MEM_MAPPED, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE,
        PAGE_EXECUTE_WRITECOPY, PAGE_READWRITE, PAGE_WRITECOPY, VirtualQuery,
    };
    let mut private: HashMap<usize, u64> = HashMap::new();
    let mut mapped = 0u64;
    let mut image: HashMap<usize, u64> = HashMap::new();
    let mut at = 0usize;
    loop {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a MEMORY_BASIC_INFORMATION; any address is valid to query.
        let n = unsafe {
            VirtualQuery(at as *const _, &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>())
        };
        if n == 0 || info.RegionSize == 0 {
            break;
        }
        if info.State == MEM_COMMIT {
            let size = info.RegionSize as u64;
            let writable = matches!(
                info.Protect & 0xff,
                PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
            );
            match info.Type {
                MEM_PRIVATE => *private.entry(info.AllocationBase as usize).or_default() += size,
                MEM_MAPPED => mapped += size,
                MEM_IMAGE if writable => *image.entry(info.AllocationBase as usize).or_default() += size,
                _ => {}
            }
        }
        let Some(next) = (info.BaseAddress as usize).checked_add(info.RegionSize) else { break };
        at = next;
    }
    let total_private: u64 = private.values().sum();
    let mut big: Vec<u64> = private.values().copied().filter(|&b| b >= 1 << 20).collect();
    big.sort_unstable_by(|a, b| b.cmp(a));
    let mut out = format!(
        "Private commit: {} in {} allocations; mapped: {}\nLargest private allocations: {}\n",
        mb(total_private),
        private.len(),
        mb(mapped),
        big.iter().take(16).map(|&b| mb(b)).collect::<Vec<_>>().join(", "),
    );
    let mut dlls: Vec<(String, u64)> = image
        .into_iter()
        .map(|(base, bytes)| {
            let mut name = [0u16; 260];
            // SAFETY: `base` is a loaded module's base address; `name` holds 260 wide chars.
            let len = unsafe { GetModuleFileNameW(base as _, name.as_mut_ptr(), name.len() as u32) };
            let path = String::from_utf16_lossy(&name[..len as usize]);
            let file = path.rsplit(['\\', '/']).next().unwrap_or("?").to_string();
            (file, bytes)
        })
        .collect();
    dlls.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    out.push_str("Writable DLL pages: ");
    out.push_str(
        &dlls.iter().take(10).map(|(n, b)| format!("{n} {}", mb(*b))).collect::<Vec<_>>().join(", "),
    );
    out.push('\n');
    out
}

#[cfg(not(windows))]
fn regions() -> String {
    String::new()
}

/// "12.3 MB".
pub fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}
