//! Finding the running game in the process list.

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::game_names::is_game;

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
        .find(|(_, process)| {
            let program = process.cmd().first().and_then(|p| p.to_str());
            is_game(process.name().to_str(), program)
        })
        .map(|(pid, _)| pid.as_u32())
}
