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

#[cfg(test)]
mod tests {
    use super::url;

    #[test]
    fn url_carries_the_match_and_the_submitter() {
        assert_eq!(
            url(101, Some(7)),
            "https://deadchaps.gg/api/match/101/populate?username=ingest-tool:7"
        );
        assert_eq!(
            url(101, None),
            "https://deadchaps.gg/api/match/101/populate"
        );
    }
}
