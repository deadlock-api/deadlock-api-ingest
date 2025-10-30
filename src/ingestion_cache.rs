use dashmap::DashMap;
use std::sync::OnceLock;

/// Global cache to track successfully ingested salts.
/// Key is the `match_id`, value is a tuple of `(has_metadata, has_replay)`.
static INGESTION_CACHE: OnceLock<DashMap<u64, (bool, bool)>> = OnceLock::new();

/// Get or initialize the global ingestion cache.
fn get_cache() -> &'static DashMap<u64, (bool, bool)> {
    INGESTION_CACHE.get_or_init(DashMap::new)
}

/// Check if a salt has already been ingested.
/// Returns true if the specific salt type (metadata or replay) has been ingested for this `match_id`.
pub(crate) fn is_ingested(match_id: u64, is_metadata: bool) -> bool {
    if let Some(entry) = get_cache().get(&match_id) {
        let (has_metadata, has_replay) = *entry;
        if is_metadata {
            has_metadata
        } else {
            has_replay
        }
    } else {
        false
    }
}

/// Mark a salt as successfully ingested.
/// This should only be called after successful ingestion.
pub(crate) fn mark_ingested(match_id: u64, is_metadata: bool) {
    get_cache()
        .entry(match_id)
        .and_modify(|entry| {
            if is_metadata {
                entry.0 = true;
            } else {
                entry.1 = true;
            }
        })
        .or_insert(if is_metadata {
            (true, false)
        } else {
            (false, true)
        });

    // Prevent unbounded growth - clear cache if it gets too large
    let cache = get_cache();
    if cache.len() > 10_000 {
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_operations() {
        let match_id = 12345678;

        // Initially not ingested
        assert!(!is_ingested(match_id, true));
        assert!(!is_ingested(match_id, false));

        // Mark metadata as ingested
        mark_ingested(match_id, true);
        assert!(is_ingested(match_id, true));
        assert!(!is_ingested(match_id, false));

        // Mark replay as ingested
        mark_ingested(match_id, false);
        assert!(is_ingested(match_id, true));
        assert!(is_ingested(match_id, false));
    }
}
