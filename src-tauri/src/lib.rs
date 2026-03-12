mod commands;
mod library;
mod models;
mod perk_index;
mod protocol;

use commands::*;
use library::load_library;
use models::SessionStore;
use perk_index::*;
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
            compress_library_cover_images,
            resolve_cover_image_src,
            resolve_local_image_src,
            start_download_project,
            start_download_catalog_entry,
            start_overwrite_catalog_entry,
            start_apply_oversize_project_action,
            remove_project,
            update_project,
            apply_oversize_project_action,
            get_project_json,
            scan_folder,
            start_scan_folder,
            get_viewers,
            open_viewer_window,
            get_perk_index_status,
            start_perk_index_task,
            sync_perk_index,
            rebuild_perk_index,
            search_perks,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
