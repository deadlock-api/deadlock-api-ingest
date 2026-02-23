use std::collections::HashMap;
use std::fs;
use std::sync::OnceLock;

const STEAM_ID_64_IDENT: u64 = 76561197960265728;

static CURRENT_STEAM_ID3: OnceLock<Option<u32>> = OnceLock::new();

/// Returns the current Steam user's account ID (`SteamID3` = ID64 - ident).
pub(crate) fn current_steam_id3() -> Option<u32> {
    *CURRENT_STEAM_ID3.get_or_init(|| {
        let id64 = get_current_steam_id64()?;
        u32::try_from(id64 - STEAM_ID_64_IDENT).ok()
    })
}

/// Get the currently logged-in Steam user's ID64 by parsing `loginusers.vdf`.
///
/// Steam marks the active user with `"MostRecent" "1"` in this file.
fn get_current_steam_id64() -> Option<u64> {
    let steam_dir = steamlocate::SteamDir::locate().ok()?;
    let vdf_path = steam_dir.path().join("config").join("loginusers.vdf");
    let content = fs::read_to_string(&vdf_path).ok()?;

    parse_login_users(&content)
        .into_iter()
        .find(|u| u.most_recent)
        .map(|u| u.steam_id)
}

#[derive(Debug)]
struct ParsedUser {
    steam_id: u64,
    most_recent: bool,
}

fn parse_login_users(content: &str) -> Vec<ParsedUser> {
    let mut users = Vec::new();
    let mut lines = content.lines().peekable();

    // Skip until we're inside the top-level "users" block
    for line in lines.by_ref() {
        if line.trim() == "{" {
            break;
        }
    }

    // Each user block: "76561198xxxxx" { ... }
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed == "}" {
            break; // end of top-level block
        }

        // Try to read a steam ID (quoted string on its own line)
        if let Some(steam_id) = extract_quoted(trimmed).and_then(|s| s.parse::<u64>().ok()) {
            // Next line should be "{"
            if lines.peek().map(|l| l.trim()) == Some("{") {
                lines.next();
                let props = parse_block(&mut lines);
                users.push(ParsedUser {
                    steam_id,
                    most_recent: props.get("MostRecent").is_some_and(|v| v == "1"),
                });
            }
        }
    }

    users
}

fn parse_block<'a>(lines: &mut impl Iterator<Item = &'a str>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "}" {
            break;
        }
        // Lines look like:  "Key"		"Value"
        let _parts: Vec<&str> = trimmed.splitn(2, '"').collect::<Vec<_>>();
        // Reparse: find all quoted strings
        let quoted: Vec<String> = extract_all_quoted(trimmed);
        if quoted.len() >= 2 {
            map.insert(quoted[0].clone(), quoted[1].clone());
        }
    }
    map
}

fn extract_quoted(s: &str) -> Option<String> {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        Some(s[1..s.len() - 1].to_string())
    } else {
        None
    }
}

fn extract_all_quoted(s: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut in_quote = false;
    let mut current = String::new();
    for ch in s.chars() {
        if ch == '"' {
            if in_quote {
                results.push(core::mem::take(&mut current));
            }
            in_quote = !in_quote;
        } else if in_quote {
            current.push(ch);
        }
    }
    results
}
