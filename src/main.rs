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
mod error;
mod http;
mod http_listener;
mod packet;
mod stream;
mod utils;

use crate::http_listener::{HttpListener, PlatformListener};

fn main() {
    loop {
        if let Err(e) = PlatformListener.listen() {
            println!("Error in HTTP listener: {e:?}");
        }
        std::thread::sleep(core::time::Duration::from_secs(10));
    }
}
