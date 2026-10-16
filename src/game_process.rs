//! Finding the running game in the process list.

use std::ffi::{OsStr, OsString};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

const GAME_PROCESS_NAMES: &[&str] = &["project8.exe", "deadlock.exe", "project8"];

/// The running game's pid, if any.
pub(crate) fn find_pid(sys: &mut System) -> Option<u32> {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    sys.processes()
        .iter()
        .find(|(_, process)| is_game(process.name(), process.cmd().first()))
        .map(|(pid, _)| pid.as_u32())
}

/// Under Proton the game renames its main thread, which Linux reports as the process name;
/// the program Proton started (`Z:\...\deadlock.exe`) still names it.
fn is_game(name: &OsStr, program: Option<&OsString>) -> bool {
    let program = program
        .and_then(|p| p.to_str())
        .and_then(|p| p.rsplit(['\\', '/']).next());
    GAME_PROCESS_NAMES.iter().any(|exe| {
        name.eq_ignore_ascii_case(exe) || program.is_some_and(|p| p.eq_ignore_ascii_case(exe))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is(name: &str, program: Option<&str>) -> bool {
        is_game(OsStr::new(name), program.map(OsString::from).as_ref())
    }

    #[test]
    fn the_game_by_its_name_or_the_program_proton_started() {
        assert!(is("deadlock.exe", None));
        assert!(is("Project8.exe", None));
        assert!(is(
            "MainThrd",
            Some(r"Z:\games\Deadlock\game\bin\win64\deadlock.exe")
        ));
        assert!(!is("MainThrd", Some(r"Z:\games\deadlock.exe.bak")));
        assert!(!is("python3", Some("/usr/bin/python3")));
    }
}
