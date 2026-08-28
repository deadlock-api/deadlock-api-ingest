use crate::notifier::Target;

fn url(match_id: u64, user: Option<u32>) -> String {
    match user {
        Some(id) => {
            format!("https://deadchaps.gg/api/match/{match_id}/populate?username=ingest-tool:{id}")
        }
        None => format!("https://deadchaps.gg/api/match/{match_id}/populate"),
    }
}

pub(crate) static TARGET: Target = Target::new("DeadChaps", url);
