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
#![allow(clippy::missing_errors_doc)]

#[cfg(target_os = "linux")]
mod http_listener_linux;
#[cfg(target_os = "windows")]
mod http_listener_win;
pub(crate) mod utils;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tracing::{error, info, warn};

#[cfg(target_os = "linux")]
use http_listener_linux::listen;
#[cfg(target_os = "windows")]
use http_listener_win::listen;

pub fn run() -> anyhow::Result<()> {
    std::thread::spawn(move || {
        loop {
            if let Err(e) = listen() {
                error!("Error in HTTP listener: {e}");
            }
            std::thread::sleep(core::time::Duration::from_secs(1));
        }
    });
    tauri::Builder::default()
        .setup(move |app| {
            setup_system_tray(app)?;
            Ok(())
        })
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .run(tauri::generate_context!())?;
    Ok(())
}

fn setup_system_tray(app: &tauri::App) -> tauri::Result<()> {
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&quit_item])?;

    // Build the tray icon with menu and event handler
    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false) // Only show menu on right-click
        .on_menu_event(move |app_handle, event| match event.id.as_ref() {
            "quit" => {
                info!("Quit menu item clicked");
                app_handle.exit(0);
            }
            _ => {
                warn!("Unhandled menu item: {:?}", event.id);
            }
        })
        .build(app)?;

    Ok(())
}
