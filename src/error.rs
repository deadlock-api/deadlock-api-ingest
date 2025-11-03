use core::fmt::Debug;
use core::fmt::Display;

pub(crate) enum Error {
    MatchIdTooLarge,
    FailedToIngest(String),
    Ureq(ureq::Error),
}

impl core::error::Error for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl Debug for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::MatchIdTooLarge => write!(f, "Match ID too large"),
            Error::FailedToIngest(s) => write!(f, "Failed to ingest: {s}"),
            Error::Ureq(e) => write!(f, "Ureq error: {e:?}"),
        }
    }
}
