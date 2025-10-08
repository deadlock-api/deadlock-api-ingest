use core::fmt::Debug;
use core::fmt::Display;

pub(crate) enum Error {
    MatchIdTooLarge,
    FailedToIngest(String),
    Ureq(ureq::Error),
    #[cfg(target_os = "linux")]
    PCap(pcap::Error),
    #[cfg(target_os = "linux")]
    NoDeviceFound,
    #[cfg(target_os = "windows")]
    PktMon(std::io::Error),
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
            #[cfg(target_os = "linux")]
            Error::PCap(e) => write!(f, "PCap error: {e:?}"),
            #[cfg(target_os = "linux")]
            Error::NoDeviceFound => write!(f, "No device found"),
            #[cfg(target_os = "windows")]
            Error::PktMon(e) => write!(f, "PktMon error: {e:?}"),
        }
    }
}
