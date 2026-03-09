mod commands;
mod library;
mod models;
mod protocol;

use commands::*;
use library::load_library;
use models::SessionStore;
use std::collections::HashMap;
use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let library = load_library();
    let sessions: SessionStore = Mutex::new(HashMap::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(library))
        .manage(sessions)
        .register_uri_scheme_protocol("cyoaview", |ctx, request| {
            protocol::handle(ctx.app_handle(), ctx.webview_label(), request)
        })
        .invoke_handler(tauri::generate_handler![
            get_library,
            add_project,
            clear_library,
            resolve_cover_image_src,
            start_download_project,
            start_download_catalog_entry,
            start_apply_oversize_project_action,
            remove_project,
            update_project,
            apply_oversize_project_action,
            get_project_json,
            scan_folder,
            get_viewers,
            open_viewer_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
