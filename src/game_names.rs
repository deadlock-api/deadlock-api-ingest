//! Telling Deadlock's process apart; shared with the `deadlock-api-ingest-memgrant` helper.

const GAME_PROCESS_NAMES: &[&str] = &["project8.exe", "deadlock.exe", "project8"];

/// Whether a process with this name, started as `program`, is the game. Under Proton the
/// game renames its main thread, which Linux reports as the process name; the program Proton
/// started (`Z:\...\deadlock.exe`) still names it.
pub(crate) fn is_game(name: Option<&str>, program: Option<&str>) -> bool {
    let program = program.and_then(|p| p.rsplit(['\\', '/']).next());
    GAME_PROCESS_NAMES.iter().any(|exe| {
        name.is_some_and(|n| n.eq_ignore_ascii_case(exe))
            || program.is_some_and(|p| p.eq_ignore_ascii_case(exe))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_by_its_name_or_the_program_proton_started() {
        assert!(is_game(Some("deadlock.exe"), None));
        assert!(is_game(Some("Project8.exe"), None));
        assert!(is_game(
            Some("MainThrd"),
            Some(r"Z:\games\Deadlock\game\bin\win64\deadlock.exe")
        ));
        assert!(!is_game(
            Some("MainThrd"),
            Some(r"Z:\games\deadlock.exe.bak")
        ));
        assert!(!is_game(Some("python3"), Some("/usr/bin/python3")));
        assert!(!is_game(None, None));
    }
}
