//! Where Sigil's memory goes, for Settings > About: the Rust heap (counted by
//! `procwatch::alloc::Counting`, the global allocator), the decoded images the app holds, and
//! the whole process as the OS counts it (Task Manager's "Memory", Activity Monitor's).
//! The difference between the process and the heap is the graphics driver, the window's
//! buffers and system libraries.

use crate::app::State;

pub struct Reading {
    /// Live bytes the Rust allocator handed out.
    pub heap: Option<u64>,
    /// The process's private memory (Windows commit, macOS physical footprint).
    pub process: Option<u64>,
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
    Reading { heap, process: process_bytes(), images: (count, bytes) }
}

#[cfg(windows)]
fn process_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    // SAFETY: the pseudo handle needs no closing; `counters` is `size` bytes.
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            size,
        )
    };
    (ok != 0).then_some(counters.PrivateUsage as u64)
}

#[cfg(target_os = "macos")]
fn process_bytes() -> Option<u64> {
    procwatch::usage().footprint
}

#[cfg(not(any(windows, target_os = "macos")))]
fn process_bytes() -> Option<u64> {
    None
}

/// "12.3 MB".
pub fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}
