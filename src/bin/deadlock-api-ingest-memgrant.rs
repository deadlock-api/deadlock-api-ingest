//! `deadlock-api-ingest-memgrant <pid>`: opens Deadlock's memory read-only and passes the
//! open file to the caller over the Unix socket on stdout.
//!
//! Most distributions only let a process open another's memory if it started it (Yama
//! `ptrace_scope` 1), and Steam starts the game. This helper is installed root-owned with
//! `cap_sys_ptrace` so the ingest tool needn't hold that right itself: Linux checks it when
//! the file is opened, not when it is read. It refuses any process that isn't Deadlock or
//! doesn't belong to the user running it.

#![deny(unsafe_code)]
#![deny(clippy::all, clippy::pedantic)]

use std::process::ExitCode;

#[cfg(target_os = "linux")]
#[path = "../game_names.rs"]
mod game_names;

fn main() -> ExitCode {
    match grant() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("deadlock-api-ingest-memgrant: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn grant() -> Result<(), String> {
    Err("only needed on Linux".to_owned())
}

#[cfg(target_os = "linux")]
fn grant() -> Result<(), String> {
    use rustix::fs::{Mode, OFlags, open};
    use std::os::fd::AsFd;

    let pid: u32 = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .ok_or("usage: deadlock-api-ingest-memgrant <pid>")?;
    // Everything is opened relative to this directory, so it all belongs to the same process
    // even if the pid is reused meanwhile.
    let proc_dir = open(
        format!("/proc/{pid}"),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| format!("no process {pid}: {e}"))?;

    let mem = rustix::fs::openat(
        &proc_dir,
        "mem",
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| format!("cannot open the memory of process {pid}: {e}"))?;

    // Checked after opening: a process that gained privileges before is refused, and one
    // that gains them later (a setuid exec) leaves us its old, unprivileged memory.
    let (uid, gid) = (
        rustix::process::getuid().as_raw(),
        rustix::process::getgid().as_raw(),
    );
    // The kernel makes a process's /proc entries root's while it isn't dumpable, e.g. after
    // dropping setuid privileges, as ptrace would refuse it.
    let owner =
        rustix::fs::fstat(&proc_dir).map_err(|e| format!("cannot stat process {pid}: {e}"))?;
    let status = read_at(&proc_dir, "status")?;
    if owner.st_uid != uid || !all_ids(&status, "Uid:", uid) || !all_ids(&status, "Gid:", gid) {
        return Err(format!("process {pid} is not the caller's alone"));
    }
    let name = read_at(&proc_dir, "comm")?;
    let cmdline = read_at(&proc_dir, "cmdline")?;
    if !game_names::is_game(Some(name.trim_end()), cmdline.split('\0').next()) {
        return Err(format!("process {pid} is not Deadlock"));
    }

    send_fd(mem.as_fd()).map_err(|e| format!("cannot pass the memory on: {e}"))
}

#[cfg(target_os = "linux")]
fn read_at(dir: &std::os::fd::OwnedFd, name: &str) -> Result<String, String> {
    use rustix::fs::{Mode, OFlags, openat};
    use std::io::Read;

    let mut text = String::new();
    openat(dir, name, OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty())
        .map(std::fs::File::from)
        .map_err(std::io::Error::from)
        .and_then(|mut file| file.read_to_string(&mut text))
        .map_err(|e| format!("cannot read /proc/<pid>/{name}: {e}"))?;
    Ok(text)
}

/// Whether every id (real, effective, saved, filesystem) on the `field` line of
/// `/proc/<pid>/status` is `id`.
#[cfg(target_os = "linux")]
fn all_ids(status: &str, field: &str, id: u32) -> bool {
    status
        .lines()
        .find_map(|line| line.strip_prefix(field))
        .is_some_and(|ids| {
            let ids: Vec<&str> = ids.split_whitespace().collect();
            ids.len() == 4 && ids.iter().all(|v| v.parse() == Ok(id))
        })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::all_ids;

    #[test]
    fn a_process_counts_as_the_callers_only_with_all_its_ids() {
        let status =
            "Name:\tMainThrd\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n";
        assert!(all_ids(status, "Uid:", 1000));
        assert!(all_ids(status, "Gid:", 1000));
        let setuid = "Uid:\t1000\t0\t0\t0\n";
        assert!(!all_ids(setuid, "Uid:", 1000));
        assert!(!all_ids("Name:\tx\n", "Uid:", 1000));
    }
}

#[cfg(target_os = "linux")]
fn send_fd(fd: std::os::fd::BorrowedFd<'_>) -> std::io::Result<()> {
    use core::mem::MaybeUninit;
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};

    let fds = [fd];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !control.push(SendAncillaryMessage::ScmRights(&fds)) {
        return Err(std::io::Error::other("control buffer too small"));
    }
    sendmsg(
        std::io::stdout(),
        &[std::io::IoSlice::new(&[0])],
        &mut control,
        SendFlags::empty(),
    )?;
    Ok(())
}
