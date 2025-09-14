#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::correctness)]
#![deny(clippy::suspicious)]
#![deny(clippy::style)]
#![deny(clippy::complexity)]
#![deny(clippy::perf)]
#![deny(clippy::pedantic)]
#![deny(clippy::std_instead_of_core)]
#![allow(clippy::unreadable_literal)]
pub(crate) mod utils;

#[cfg(target_os = "linux")]
mod http_listener_linux;
#[cfg(target_os = "windows")]
mod http_listener_win;

#[cfg(target_os = "linux")]
use http_listener_linux::listen;
#[cfg(target_os = "windows")]
use http_listener_win::listen;

use tracing::error;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new(
        "debug,hyper_util=warn,reqwest=warn,rustls=warn",
    ));
    let fmt_layer = tracing_subscriber::fmt::layer();

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(env_filter)
        .init();
}
fn main() -> anyhow::Result<()> {
    init_tracing();

    loop {
        if let Err(e) = listen() {
            error!("Error in HTTP listener: {e}");
        }
        std::thread::sleep(core::time::Duration::from_secs(10));
    }
}
