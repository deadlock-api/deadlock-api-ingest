use crate::notifier::Target;

fn url(match_id: u64, user: Option<u32>) -> String {
    match user {
        Some(id) => {
            format!("https://statlocker.gg/api/match/{match_id}/populate?username=ingest-tool:{id}")
        }
        None => format!("https://statlocker.gg/api/match/{match_id}/populate"),
    }
}

pub(crate) static TARGET: Target = Target::new("Statlocker", url);
