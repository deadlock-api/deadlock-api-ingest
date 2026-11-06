use core::fmt::Debug;
use core::fmt::Display;

/// How much of a rejection's body goes into the message.
const MAX_BODY_CHARS: usize = 300;

pub(crate) enum Error {
    MatchIdTooLarge,
    /// The API answered with a non-2xx status.
    Rejected {
        status: u16,
        body: String,
    },
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
            Error::Rejected { status, body } => {
                write!(f, "API answered {status}")?;
                // Error pages from a proxy in front of the API can be long HTML.
                let body = body.trim();
                match body.char_indices().nth(MAX_BODY_CHARS) {
                    None if body.is_empty() => Ok(()),
                    None => write!(f, ": {body}"),
                    Some((end, _)) => write!(f, ": {}…", &body[..end]),
                }
            }
            Error::Ureq(e) => write!(f, "Ureq error: {e:?}"),
        }
    }
}
