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

    let status = read_at(&proc_dir, "status")?;
    if real_uid(&status) != Some(rustix::process::getuid().as_raw()) {
        return Err(format!("process {pid} belongs to another user"));
    }
    let name = read_at(&proc_dir, "comm")?;
    let cmdline = read_at(&proc_dir, "cmdline")?;
    if !game_names::is_game(Some(name.trim_end()), cmdline.split('\0').next()) {
        return Err(format!("process {pid} is not Deadlock"));
    }

    let mem = rustix::fs::openat(
        &proc_dir,
        "mem",
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| format!("cannot open the memory of process {pid}: {e}"))?;
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

/// The real uid on the `Uid:` line of `/proc/<pid>/status`.
#[cfg(target_os = "linux")]
fn real_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
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
