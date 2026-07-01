//! Rolling-window fetch quota — the hard cap on Steam GC match-salt fetches.
//! Pure and clock-injected so it unit-tests without a store or real clock;
//! persistence lives in [`super::store`].

pub(crate) const FETCH_QUOTA_LIMIT: usize = 40;
pub(crate) const FETCH_QUOTA_WINDOW_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone)]
pub(crate) struct QuotaWindow {
    hits: Vec<i64>,
    limit: usize,
    window_secs: i64,
}

impl QuotaWindow {
    pub(crate) fn new(hits: Vec<i64>, limit: usize, window_secs: i64) -> Self {
        Self {
            hits,
            limit,
            window_secs,
        }
    }

    fn prune(&mut self, now: i64) {
        let cutoff = now - self.window_secs;
        self.hits.retain(|&t| t > cutoff);
    }

    pub(crate) fn remaining(&self, now: i64) -> usize {
        let cutoff = now - self.window_secs;
        let used = self.hits.iter().filter(|&&t| t > cutoff).count();
        self.limit.saturating_sub(used)
    }

    /// On success records `now`; caller must persist [`Self::snapshot`]. Nothing
    /// mutates on failure.
    pub(crate) fn try_consume(&mut self, now: i64) -> bool {
        self.prune(now);
        if self.hits.len() >= self.limit {
            return false;
        }
        self.hits.push(now);
        true
    }

    pub(crate) fn snapshot(&self) -> Vec<i64> {
        self.hits.clone()
    }

    /// For when Steam itself rate-limits us: treat the window as fully used from `now`.
    pub(crate) fn exhaust(&mut self, now: i64) {
        self.prune(now);
        let needed = self.limit.saturating_sub(self.hits.len());
        self.hits.extend(core::iter::repeat_n(now, needed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: usize = 40;
    const WINDOW: i64 = 24 * 60 * 60;

    fn window() -> QuotaWindow {
        QuotaWindow::new(Vec::new(), LIMIT, WINDOW)
    }

    #[test]
    fn allows_up_to_limit_then_blocks() {
        let mut q = window();
        let now = 1_000_000;
        for _ in 0..LIMIT {
            assert!(q.try_consume(now));
        }
        assert_eq!(q.remaining(now), 0);
        assert!(!q.try_consume(now));
    }

    #[test]
    fn frees_capacity_as_hits_age_out() {
        let mut q = window();
        let start = 1_000_000;
        for _ in 0..LIMIT {
            assert!(q.try_consume(start));
        }
        assert!(!q.try_consume(start + WINDOW - 1));
        let later = start + WINDOW + 1;
        assert_eq!(q.remaining(later), LIMIT);
        assert!(q.try_consume(later));
    }

    #[test]
    fn exhaust_blocks_for_a_full_window_from_now() {
        let mut q = window();
        let now = 1_000_000;
        q.try_consume(now);
        q.exhaust(now);
        assert_eq!(q.remaining(now), 0);
        assert_eq!(q.snapshot().len(), LIMIT);
        assert_eq!(q.remaining(now + WINDOW - 1), 0);
        assert_eq!(q.remaining(now + WINDOW + 1), LIMIT);
    }
}
