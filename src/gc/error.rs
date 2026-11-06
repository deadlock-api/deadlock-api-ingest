use core::fmt::{Debug, Display};

// Messages never include the refresh token / GC session (they are secrets).
pub(crate) enum GcError {
    /// No usable Steam session on this machine (logged out, "remember me" off,
    /// or the `ConnectCache` blob could not be decrypted).
    AuthUnavailable(String),
    /// Steam or a GC job failed: no CM connection, or the GC answered a request badly.
    GcUnavailable(String),
    /// The GC never welcomed the session (the account doesn't own the game, or the GC is
    /// down). Says nothing about the request it was for, and every further request this
    /// pass would fail the same way after a full timeout.
    GcHandshakeFailed(String),
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
            GcError::GcHandshakeFailed(s) => write!(f, "Steam GC did not answer: {s}"),
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
