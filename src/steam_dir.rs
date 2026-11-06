//! Which Steam install to use when there are several.
//!
//! `steamlocate::locate()` returns the first install it finds, and on Linux it checks Flatpak
//! before native. `flatpak uninstall` keeps the app's data unless given `--delete-data`, so a
//! leftover Flatpak dir would win over the native install that's actually in use.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use tracing::{info, warn};

struct SteamDirs {
    /// Every install found, the picked one first.
    all: Vec<PathBuf>,
}

static STEAM_DIRS: OnceLock<SteamDirs> = OnceLock::new();

fn steam_dirs() -> &'static SteamDirs {
    STEAM_DIRS.get_or_init(|| {
        let found = match steamlocate::locate_all() {
            Ok(dirs) => dedup(dirs.iter().map(|d| d.path().to_path_buf())),
            Err(e) => {
                warn!("Could not find a Steam directory: {e}");
                Vec::new()
            }
        };
        let with_mtime = found
            .into_iter()
            .map(|dir| {
                let signed_in = last_sign_in(&dir);
                (dir, signed_in)
            })
            .collect::<Vec<_>>();
        if with_mtime.len() > 1 {
            for (dir, signed_in) in &with_mtime {
                info!(
                    "Found Steam directory {} (loginusers.vdf {})",
                    dir.display(),
                    signed_in.map_or_else(|| "missing".to_owned(), describe_age)
                );
            }
        }
        let all = order_by_last_sign_in(with_mtime);
        if let Some(picked) = all.first() {
            info!("Using Steam directory: {}", picked.display());
        }
        SteamDirs { all }
    })
}

/// The Steam install to read the signed-in accounts and their sessions from: the one whose
/// `config/loginusers.vdf` changed last, since Steam rewrites it when it signs in.
pub(crate) fn primary() -> Option<&'static Path> {
    steam_dirs().all.first().map(PathBuf::as_path)
}

/// Every Steam install found, [`primary`] first.
pub(crate) fn all() -> &'static [PathBuf] {
    &steam_dirs().all
}

fn last_sign_in(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join("config").join("loginusers.vdf"))
        .and_then(|m| m.modified())
        .ok()
}

/// `steamlocate` only resolves one level of symlink, so `~/.steam/root` and
/// `~/.local/share/Steam` can both show up for one install.
fn dedup(dirs: impl Iterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = Vec::new();
    let mut unique = Vec::new();
    for dir in dirs {
        let resolved = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        if !seen.contains(&resolved) {
            seen.push(resolved);
            unique.push(dir);
        }
    }
    unique
}

/// Latest sign-in first; installs with no `loginusers.vdf` last. Ties keep `steamlocate`'s order.
fn order_by_last_sign_in(mut dirs: Vec<(PathBuf, Option<SystemTime>)>) -> Vec<PathBuf> {
    // Stable sort, and `None` sorts below every `Some`.
    dirs.sort_by_key(|(_, signed_in)| core::cmp::Reverse(*signed_in));
    dirs.into_iter().map(|(dir, _)| dir).collect()
}

fn describe_age(time: SystemTime) -> String {
    match SystemTime::now().duration_since(time) {
        Ok(age) if age.as_secs() < 2 * 3600 => format!("changed {} min ago", age.as_secs() / 60),
        Ok(age) => format!("changed {} h ago", age.as_secs() / 3600),
        Err(_) => "changed in the future".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    #[test]
    fn latest_sign_in_wins_over_steamlocate_order() {
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let new = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);
        let ordered = order_by_last_sign_in(vec![
            (PathBuf::from("flatpak"), Some(old)),
            (PathBuf::from("empty"), None),
            (PathBuf::from("native"), Some(new)),
        ]);
        assert_eq!(
            ordered,
            [
                PathBuf::from("native"),
                PathBuf::from("flatpak"),
                PathBuf::from("empty")
            ]
        );
    }

    #[test]
    fn ties_keep_steamlocate_order() {
        let ordered = order_by_last_sign_in(vec![
            (PathBuf::from("first"), None),
            (PathBuf::from("second"), None),
        ]);
        assert_eq!(ordered, [PathBuf::from("first"), PathBuf::from("second")]);
    }

    #[test]
    fn symlinked_duplicates_are_dropped() {
        let root = std::env::temp_dir().join(format!("steam-dir-dedup-{}", std::process::id()));
        let real = root.join("real");
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, root.join("link")).unwrap();
        #[cfg(unix)]
        let input = [real.clone(), root.join("link"), root.join("other")];
        #[cfg(not(unix))]
        let input = [real.clone(), real.clone(), root.join("other")];
        let unique = dedup(input.into_iter());
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(unique, [real, root.join("other")]);
    }
}
