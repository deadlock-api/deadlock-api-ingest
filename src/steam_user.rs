use std::collections::HashMap;
use std::fs;

pub(crate) const STEAM_ID_64_IDENT: u64 = 76561197960265728;

/// Returns the current Steam user's account ID (`SteamID3` = ID64 - ident). Read on every
/// call: Steam can switch accounts while the watcher runs.
pub(crate) fn current_steam_id3() -> Option<u32> {
    let id64 = get_current_steam_id64()?;
    u32::try_from(id64.checked_sub(STEAM_ID_64_IDENT)?).ok()
}

/// Get the currently logged-in Steam user's ID64 by parsing `loginusers.vdf`.
fn get_current_steam_id64() -> Option<u64> {
    let vdf_path = crate::steam_dir::primary()?
        .join("config")
        .join("loginusers.vdf");
    let content = fs::read_to_string(&vdf_path).ok()?;
    signed_in_user(&content)
}

/// Older Steam clients mark the signed-in user with `"MostRecent" "1"`; current ones no
/// longer write that key and mark it with `"AutoLogin" "1"` instead.
fn signed_in_user(loginusers_vdf: &str) -> Option<u64> {
    let users = parse_login_users(loginusers_vdf);
    users
        .iter()
        .find(|u| u.most_recent)
        .or_else(|| users.iter().find(|u| u.auto_login))
        .map(|u| u.steam_id)
}

#[derive(Debug)]
struct ParsedUser {
    steam_id: u64,
    most_recent: bool,
    auto_login: bool,
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
                    most_recent: props.get("mostrecent").is_some_and(|v| v == "1"),
                    auto_login: props.get("autologin").is_some_and(|v| v == "1"),
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
        // Compared case-insensitively: Steam has written both "Timestamp" and "timestamp".
        if quoted.len() >= 2 {
            map.insert(quoted[0].to_ascii_lowercase(), quoted[1].clone());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signed_in_user() {
        // Current clients: no MostRecent; the signed-in account has AutoLogin 1.
        let current = r#""users"
{
	"76561198000000001"
	{
		"AccountName"		"a"
		"AutoLogin"		"0"
	}
	"76561198000000002"
	{
		"AccountName"		"b"
		"AutoLogin"		"1"
	}
}"#;
        assert_eq!(signed_in_user(current), Some(76561198000000002));

        // Older clients: MostRecent wins.
        let older = r#""users"
{
	"76561198000000001"
	{
		"MostRecent"		"1"
		"AutoLogin"		"0"
	}
	"76561198000000002"
	{
		"MostRecent"		"0"
		"AutoLogin"		"1"
	}
}"#;
        assert_eq!(signed_in_user(older), Some(76561198000000001));

        let lowercase =
            "\"users\"\n{\n\t\"76561198000000003\"\n\t{\n\t\t\"autologin\"\t\t\"1\"\n\t}\n}";
        assert_eq!(signed_in_user(lowercase), Some(76561198000000003));

        let nobody =
            "\"users\"\n{\n\t\"76561198000000001\"\n\t{\n\t\t\"AutoLogin\"\t\t\"0\"\n\t}\n}";
        assert_eq!(signed_in_user(nobody), None);
    }
}
