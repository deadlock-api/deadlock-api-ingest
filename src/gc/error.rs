use core::fmt::{Debug, Display};

// Messages never include the refresh token / GC session (they are secrets).
pub(crate) enum GcError {
    /// No usable Steam session on this machine (logged out, "remember me" off,
    /// or the `ConnectCache` blob could not be decrypted).
    AuthUnavailable(String),
    /// The GC handshake or a GC job failed (e.g. the account does not own the game).
    GcUnavailable(String),
    /// Steam's GC answered but is throttling this account; back off, don't burn quota.
    GcRateLimited,
    /// The GC refused a friend's match history (their profile is private).
    HistoryPrivate,
    /// Another session of the account is playing a game, on this PC or another. Talking to
    /// the GC would interrupt it, so the account is skipped until the next pass.
    AccountInUse,
    /// The deadlock-api to-fetch/salts HTTP call failed.
    Api(String),
    /// Reading or writing the persisted quota/backoff store failed.
    Store(String),
}

impl core::error::Error for GcError {}

impl Display for GcError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GcError::AuthUnavailable(s) => write!(f, "Steam session unavailable: {s}"),
            GcError::GcUnavailable(s) => write!(f, "Steam GC unavailable: {s}"),
            GcError::GcRateLimited => write!(f, "Steam GC rate-limited the request"),
            GcError::HistoryPrivate => write!(f, "match history is private"),
            GcError::AccountInUse => write!(f, "the Steam account is playing a game"),
            GcError::Api(s) => write!(f, "deadlock-api request failed: {s}"),
            GcError::Store(s) => write!(f, "GC store error: {s}"),
        }
    }
}

impl Debug for GcError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Display::fmt(self, f)
    }
}
