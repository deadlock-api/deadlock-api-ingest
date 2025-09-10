use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

mod http_listener;

pub fn run() {
    std::thread::spawn(move || {
        loop {
            if let Err(e) = http_listener::listen() {
                eprintln!("Error in HTTP listener: {}", e);
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });
    tauri::Builder::default()
        .setup(move |app| {
            setup_system_tray(app)?;
            Ok(())
        })
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_system_tray(app: &tauri::App) -> tauri::Result<()> {
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&quit_item])?;

    // Build the tray icon with menu and event handler
    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false) // Only show menu on right-click
        .on_menu_event(move |app_handle, event| {
            match event.id.as_ref() {
                "quit" => {
                    println!("Quit menu item clicked");
                    app_handle.exit(0);
                }
                _ => {
                    println!("Unhandled menu item: {:?}", event.id);
                }
            }
        })
        .build(app)?;

    Ok(())
}
