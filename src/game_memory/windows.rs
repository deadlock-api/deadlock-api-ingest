//! The game opened with query and read rights only.

use core::ptr;
use std::os::windows::io::{AsRawHandle, HandleOrNull, OwnedHandle};

use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY,
    VirtualQueryEx,
};
use windows_sys::Win32::System::ProcessStatus::{
    K32QueryWorkingSetEx, PSAPI_WORKING_SET_EX_BLOCK, PSAPI_WORKING_SET_EX_INFORMATION,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};

use super::{MAX_REGION, Region};

const READABLE: [u32; 6] = [
    PAGE_READONLY,
    PAGE_READWRITE,
    PAGE_WRITECOPY,
    PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY,
];
/// The top of the user address space on 64-bit Windows.
const USER_TOP: usize = 0x7FFF_FFFF_FFFF;
const PAGE: usize = 4096;
const PAGES_PER_QUERY: usize = 1 << 16;

pub(super) struct GameProcess {
    handle: OwnedHandle,
}

impl GameProcess {
    #[allow(unsafe_code)]
    pub(super) fn open(pid: u32) -> std::io::Result<Self> {
        // The full query right lets `resident` ask which pages are in RAM; the limited one
        // still reads.
        for rights in [PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION] {
            // SAFETY: OpenProcess takes plain values and returns either NULL or a new handle
            // that nothing else owns.
            let handle = unsafe {
                HandleOrNull::from_raw_handle(OpenProcess(rights | PROCESS_VM_READ, 0, pid))
            };
            if let Ok(handle) = OwnedHandle::try_from(handle) {
                return Ok(Self { handle });
            }
        }
        Err(std::io::Error::last_os_error())
    }

    /// The game's heap, cut down to the pages in RAM where Windows says which those are:
    /// reading paged-out memory would pull it back from disk at the game's expense.
    pub(super) fn regions(&self) -> Vec<Region> {
        let regions = self.private_regions();
        self.resident(&regions).unwrap_or(regions)
    }

    #[allow(unsafe_code)]
    fn private_regions(&self) -> Vec<Region> {
        let mut out = Vec::new();
        let mut addr = 0_usize;
        while addr < USER_TOP {
            let mut info = MEMORY_BASIC_INFORMATION::default();
            // SAFETY: `info` is a writable MEMORY_BASIC_INFORMATION of the size passed. The
            // address is looked up in the game, never dereferenced here.
            let written = unsafe {
                VirtualQueryEx(
                    self.handle.as_raw_handle(),
                    ptr::without_provenance(addr),
                    &raw mut info,
                    size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if written < size_of::<MEMORY_BASIC_INFORMATION>() {
                break;
            }
            let (base, size) = (info.BaseAddress.addr(), info.RegionSize);
            if info.State == MEM_COMMIT
                && info.Type == MEM_PRIVATE
                && info.Protect & PAGE_GUARD == 0
                && READABLE.contains(&(info.Protect & 0xFF))
                && size <= MAX_REGION
            {
                out.push(Region { base, size });
            }
            match base.checked_add(size) {
                Some(next) if next > addr => addr = next,
                _ => break,
            }
        }
        out
    }

    /// The parts of `regions` in the game's working set, or `None` when Windows won't say.
    #[allow(unsafe_code)]
    fn resident(&self, regions: &[Region]) -> Option<Vec<Region>> {
        let mut out: Vec<Region> = Vec::new();
        let mut query = Vec::with_capacity(PAGES_PER_QUERY);
        for region in regions {
            let end = region.base.saturating_add(region.size);
            let mut pages = (region.base..end).step_by(PAGE);
            loop {
                query.clear();
                query.extend(pages.by_ref().take(PAGES_PER_QUERY).map(|addr| {
                    PSAPI_WORKING_SET_EX_INFORMATION {
                        VirtualAddress: ptr::without_provenance_mut(addr),
                        VirtualAttributes: PSAPI_WORKING_SET_EX_BLOCK::default(),
                    }
                }));
                if query.is_empty() {
                    break;
                }
                let bytes = u32::try_from(size_of_val(query.as_slice())).ok()?;
                // SAFETY: `query` is a writable, aligned buffer of `bytes` bytes holding the
                // addresses to ask about; the call fills in their attributes.
                let ok = unsafe {
                    K32QueryWorkingSetEx(
                        self.handle.as_raw_handle(),
                        query.as_mut_ptr().cast(),
                        bytes,
                    )
                };
                if ok == 0 {
                    return None;
                }
                for entry in &query {
                    // SAFETY: every variant of the union is plain integer bits.
                    let flags = unsafe { entry.VirtualAttributes.Flags };
                    // Bit 0: the page is valid, i.e. in RAM.
                    if flags & 1 == 0 {
                        continue;
                    }
                    let page = entry.VirtualAddress.addr();
                    match out.last_mut() {
                        Some(last) if last.base.checked_add(last.size) == Some(page) => {
                            last.size = last.size.saturating_add(PAGE);
                        }
                        _ => out.push(Region {
                            base: page,
                            size: PAGE,
                        }),
                    }
                }
            }
        }
        Some(out)
    }

    /// Reads up to `buf.len()` bytes at `addr`; 0 when the memory is gone.
    #[allow(unsafe_code)]
    pub(super) fn read(&self, addr: usize, buf: &mut [u8]) -> usize {
        let mut read = 0;
        // SAFETY: the handle has PROCESS_VM_READ, `buf` is writable for its length and `read`
        // receives the count. The address is the game's, never dereferenced here. A partial
        // copy fails with ERROR_PARTIAL_COPY but still sets `read`, so the result is unused.
        unsafe {
            ReadProcessMemory(
                self.handle.as_raw_handle(),
                ptr::without_provenance(addr),
                buf.as_mut_ptr().cast(),
                buf.len(),
                &raw mut read,
            );
        }
        read.min(buf.len())
    }
}
