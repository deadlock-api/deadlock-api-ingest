//! Under Proton the game's address space is the Windows program's, read through
//! `/proc/<pid>/mem`. With Yama's default `ptrace_scope` only the game's ancestors (the
//! launch wrapper) or a process with `CAP_SYS_PTRACE` may open it.

use std::fs::{self, File};
use std::os::unix::fs::FileExt;

use super::{MAX_REGION, Region};

pub(super) struct GameProcess {
    pid: u32,
    mem: File,
}

impl GameProcess {
    pub(super) fn open(pid: u32) -> std::io::Result<Self> {
        let mem = File::open(format!("/proc/{pid}/mem"))?;
        Ok(Self { pid, mem })
    }

    /// Readable, private, anonymous mappings: where Wine puts the game's heap.
    pub(super) fn regions(&self) -> Vec<Region> {
        fs::read_to_string(format!("/proc/{}/maps", self.pid))
            .map(|maps| heap_regions(&maps))
            .unwrap_or_default()
    }

    /// Reads up to `buf.len()` bytes at `addr`; 0 when the memory is gone.
    pub(super) fn read(&self, addr: usize, buf: &mut [u8]) -> usize {
        u64::try_from(addr).map_or(0, |offset| self.mem.read_at(buf, offset).unwrap_or(0))
    }
}

fn heap_regions(maps: &str) -> Vec<Region> {
    maps.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let (range, perms) = (parts.next()?, parts.next()?.as_bytes());
            let inode = parts.nth(2)?;
            let name = parts.next().unwrap_or_default();
            let heap = perms.first() == Some(&b'r')
                && perms.get(3) == Some(&b'p')
                && inode == "0"
                && !matches!(name, "[vvar]" | "[vvar_vclock]" | "[vdso]" | "[vsyscall]");
            if !heap {
                return None;
            }
            let (start, end) = range.split_once('-')?;
            let start = usize::from_str_radix(start, 16).ok()?;
            let size = usize::from_str_radix(end, 16).ok()?.checked_sub(start)?;
            (1..=MAX_REGION)
                .contains(&size)
                .then_some(Region { base: start, size })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_readable_private_anonymous_mappings_are_heap() {
        let maps = "\
140000000-140001000 r--p 00000000 103:02 1234 /games/deadlock/game/bin/win64/deadlock.exe
7f0000000000-7f0000100000 rw-p 00000000 00:00 0
7f0000100000-7f0000200000 ---p 00000000 00:00 0
7f0000200000-7f0000300000 rw-s 00000000 00:01 77 /memfd:wine (deleted)
7f0000300000-7f0000301000 rw-p 00000000 00:00 0 [heap]
7ffd00000000-7ffd00002000 r--p 00000000 00:00 0 [vvar]
7e0000000000-7e0080000000 rw-p 00000000 00:00 0
";
        assert_eq!(
            heap_regions(maps),
            [
                Region {
                    base: 0x7f00_0000_0000,
                    size: 0x10_0000
                },
                Region {
                    base: 0x7f00_0030_0000,
                    size: 0x1000
                },
            ]
        );
    }
}
