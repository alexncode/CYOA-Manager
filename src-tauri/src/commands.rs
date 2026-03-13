use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;

use base64::Engine;
use image::{DynamicImage, codecs::jpeg::JpegEncoder};
use regex::Regex;
use tauri::{Emitter, Manager, State};
use uuid::Uuid;
use chrono::Utc;

use crate::library::{
    clear_projects as clear_library_storage,
    cyoas_dir,
    delete_project as delete_library_project,
    insert_project as insert_library_project,
    reload_library,
    set_project_favorite as persist_project_favorite,
    set_project_viewer_preference as persist_project_viewer_preference,
    update_project as persist_project,
    update_projects as persist_projects,
};
use crate::models::{Library, Project, ProjectPatch, SessionStore, Viewer, ViewerSession};
use crate::perk_index::{clear_index_if_present, remove_project_from_index_if_present, sync_index_for_project_if_present};

pub type LibraryState = Mutex<Library>;
pub type MigrationNoticeState = Mutex<Option<String>>;
const LIBRARY_COVER_IMAGE_MAX_BYTES: usize = 60 * 1024;

#[derive(Clone, Copy)]
enum OversizeStrategy {
    Ask,
    KeepSeparate,
    Compress,
    DoNothing,
}

impl OversizeStrategy {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "ask" => Ok(Self::Ask),
            "keep-separate" => Ok(Self::KeepSeparate),
            "compress" => Ok(Self::Compress),
            "do-nothing" => Ok(Self::DoNothing),
            other => Err(format!("Invalid oversize strategy: {}", other)),
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProjectProgress {
    task_id: String,
    phase: String,
    current: usize,
    total: usize,
    image_current: usize,
    image_total: usize,
    message: String,
    done: bool,
    success: bool,
    error: Option<String>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogImportProgress {
    task_id: String,
    phase: String,
    current: usize,
    total: usize,
    message: String,
    done: bool,
    success: bool,
    error: Option<String>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OversizeActionProgress {
    task_id: String,
    project_id: String,
    phase: String,
    message: String,
    done: bool,
    success: bool,
    error: Option<String>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BulkScanProgress {
    task_id: String,
    scanned: usize,
    found: usize,
    message: String,
    done: bool,
    success: bool,
    error: Option<String>,
    paths: Vec<String>,
}

// ─── Library CRUD ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_library(state: State<LibraryState>) -> Result<Vec<Project>, String> {
    let mut lib = state.lock().map_err(|e| e.to_string())?;
    for project in lib.projects.iter_mut() {
        project.file_missing = !Path::new(&project.file_path).exists();
    }
    Ok(lib.projects.clone())
}

#[tauri::command]
pub fn take_library_migration_notice(
    state: State<MigrationNoticeState>,
) -> Result<Option<String>, String> {
    let mut notice = state.lock().map_err(|e| e.to_string())?;
    Ok(notice.take())
}

#[tauri::command]
pub fn add_project(file_path: String, state: State<LibraryState>) -> Result<Project, String> {
    add_project_from_path(file_path, &state, None)
}

#[tauri::command]
pub fn resolve_cover_image_src(
    file_path: String,
    cover_image: Option<String>,
) -> Result<Option<String>, String> {
    resolve_cover_image_source(&file_path, cover_image.as_deref())
}

#[tauri::command]
pub fn resolve_local_image_src(image_path: String) -> Result<Option<String>, String> {
    let raw_image_path = image_path.trim();
    if raw_image_path.is_empty() {
        return Ok(None);
    }

    if is_remote_or_embedded_image(raw_image_path) {
        return Ok(Some(raw_image_path.to_string()));
    }

    let image_path = PathBuf::from(raw_image_path);
    if !image_path.exists() {
        return Ok(None);
    }

    let mime = mime_guess::from_path(&image_path)
        .first_raw()
        .unwrap_or("application/octet-stream");
    let bytes = std::fs::read(&image_path).map_err(|e| e.to_string())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(Some(format!("data:{};base64,{}", mime, encoded)))
}

#[tauri::command]
pub fn compress_library_cover_images(state: State<LibraryState>) -> Result<usize, String> {
    let mut lib = state.lock().map_err(|e| e.to_string())?;
    let mut changed = 0usize;
    let mut updated_projects = Vec::new();

    for project in &lib.projects {
        let next_cover_image = compress_cover_image_for_library(
            &project.file_path,
            project.cover_image.as_deref(),
            LIBRARY_COVER_IMAGE_MAX_BYTES,
        )?;

        if next_cover_image != project.cover_image {
            let mut updated = project.clone();
            updated.cover_image = next_cover_image;
            updated_projects.push(updated);
            changed += 1;
        }
    }

    if changed > 0 {
        persist_projects(&updated_projects)?;

        for updated in updated_projects {
            if let Some(project) = lib.projects.iter_mut().find(|project| project.id == updated.id) {
                *project = updated;
            }
        }
    }

    Ok(changed)
}

#[tauri::command]
pub fn start_download_project(
    app: tauri::AppHandle,
    url: String,
    max_project_size_mb: u64,
    download_included_icc_plus_viewer: bool,
) -> Result<String, String> {
    let task_id = Uuid::new_v4().to_string();
    let app_handle = app.clone();
    let task_id_for_thread = task_id.clone();
    let limit_mb = max_project_size_mb.clamp(1, 2000);

    thread::spawn(move || {
        if let Err(error) = run_download_project(
            &app_handle,
            &task_id_for_thread,
            url,
            limit_mb,
            download_included_icc_plus_viewer,
        ) {
            emit_download_progress(
                app_handle,
                DownloadProjectProgress {
                    task_id: task_id_for_thread,
                    phase: "error".to_string(),
                    current: 1,
                    total: 1,
                    image_current: 0,
                    image_total: 0,
                    message: "Download failed".to_string(),
                    done: true,
                    success: false,
                    error: Some(error),
                },
            );
        }
    });

    Ok(task_id)
}

#[tauri::command]
pub fn start_download_catalog_entry(
    app: tauri::AppHandle,
    task_id: String,
    website_url: String,
    zip_url: String,
    project_name: String,
    max_project_size_mb: u64,
) -> Result<String, String> {
    let task_id = task_id.trim().to_string();
    if task_id.is_empty() {
        return Err("Missing task id".to_string());
    }
    let app_handle = app.clone();
    let task_id_for_thread = task_id.clone();
    let limit_mb = max_project_size_mb.clamp(1, 2000);

    thread::spawn(move || {
        if let Err(error) = run_download_catalog_entry(
            &app_handle,
            &task_id_for_thread,
            website_url,
            zip_url,
            project_name,
            limit_mb,
        ) {
            emit_catalog_import_progress(
                app_handle,
                CatalogImportProgress {
                    task_id: task_id_for_thread,
                    phase: "error".to_string(),
                    current: 100,
                    total: 100,
                    message: "Catalog import failed".to_string(),
                    done: true,
                    success: false,
                    error: Some(error),
                },
            );
        }
    });

    Ok(task_id)
}

#[tauri::command]
pub fn start_overwrite_catalog_entry(
    app: tauri::AppHandle,
    task_id: String,
    project_id: String,
    website_url: String,
    zip_url: String,
    project_name: String,
    max_project_size_mb: u64,
) -> Result<String, String> {
    let task_id = task_id.trim().to_string();
    if task_id.is_empty() {
        return Err("Missing task id".to_string());
    }

    let project_id = project_id.trim().to_string();
    if project_id.is_empty() {
        return Err("Missing project id".to_string());
    }

    let app_handle = app.clone();
    let task_id_for_thread = task_id.clone();
    let limit_mb = max_project_size_mb.clamp(1, 2000);

    thread::spawn(move || {
        if let Err(error) = run_overwrite_catalog_entry(
            &app_handle,
            &task_id_for_thread,
            &project_id,
            website_url,
            zip_url,
            project_name,
            limit_mb,
        ) {
            emit_catalog_import_progress(
                app_handle,
                CatalogImportProgress {
                    task_id: task_id_for_thread,
                    phase: "error".to_string(),
                    current: 100,
                    total: 100,
                    message: "Catalog overwrite failed".to_string(),
                    done: true,
                    success: false,
                    error: Some(error),
                },
            );
        }
    });

    Ok(task_id)
}

fn run_download_project(
    app: &tauri::AppHandle,
    task_id: &str,
    url: String,
    max_project_size_mb: u64,
    download_included_icc_plus_viewer: bool,
) -> Result<(), String> {
    emit_download_progress(
        app.clone(),
        DownloadProjectProgress {
            task_id: task_id.to_string(),
            phase: "fetching-project".to_string(),
            current: 0,
            total: 1,
            image_current: 0,
            image_total: 0,
            message: "Downloading project.json".to_string(),
            done: false,
            success: false,
            error: None,
        },
    );

    let parsed = tauri::Url::parse(url.trim()).map_err(|e| format!("Invalid URL: {}", e))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http:// and https:// URLs are supported".to_string());
    }

    if is_cyoa_cafe_host(parsed.host_str()) {
        emit_download_progress(
            app.clone(),
            DownloadProjectProgress {
                task_id: task_id.to_string(),
                phase: "resolving-source-url".to_string(),
                current: 0,
                total: 1,
                image_current: 0,
                image_total: 0,
                message: "Resolving cyoa.cafe link".to_string(),
                done: false,
                success: false,
                error: None,
            },
        );
    }

    let (bytes, base_url) = download_project_data(parsed, Some((app, task_id, false)))?;

    emit_download_progress(
        app.clone(),
        DownloadProjectProgress {
            task_id: task_id.to_string(),
            phase: "downloaded-project".to_string(),
            current: 0,
            total: 1,
            image_current: 0,
            image_total: 0,
            message: format!(
                "Downloaded project payload ({})",
                format_bytes_megabytes(bytes.len() as u64)
            ),
            done: false,
            success: false,
            error: None,
        },
    );

    let downloaded_viewer_id = if download_included_icc_plus_viewer {
        maybe_download_included_icc_plus_viewer(app, task_id, &base_url, bytes.as_ref())?
    } else {
        None
    };
    let viewer_message_suffix = if download_included_icc_plus_viewer {
        if downloaded_viewer_id.is_some() {
            " with bundled ICC+ viewer"
        } else {
            " (no bundled ICC+ viewer found on the site)"
        }
    } else {
        ""
    };

    let processed = inline_downloaded_project_images(bytes.as_ref(), &base_url, app, task_id)?;

    let dir = cyoas_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create cyoas folder: {}", e))?;
    let destination = unique_path(dir.join(download_file_name(&base_url)));

    std::fs::write(&destination, processed)
        .map_err(|e| format!("Failed to save file: {}", e))?;

    let library = app.state::<LibraryState>();
    let mut project = add_project_from_path(
        destination.to_string_lossy().to_string(),
        &library,
        normalize_source_url(&url),
    )?;
    if let Some(viewer_id) = downloaded_viewer_id {
        project = set_project_viewer_preference_after_import(&project.id, &viewer_id, &library)?;
    }
    let max_project_size_bytes = max_project_size_mb.saturating_mul(1024 * 1024);
    ensure_project_within_size_limit(&project.id, max_project_size_bytes, &library)?;

    emit_download_progress(
        app.clone(),
        DownloadProjectProgress {
            task_id: task_id.to_string(),
            phase: "done".to_string(),
            current: 1,
            total: 1,
            image_current: 1,
            image_total: 1,
            message: format!("Imported {}{}", project.name, viewer_message_suffix),
            done: true,
            success: true,
            error: None,
        },
    );

    Ok(())
}

fn run_download_catalog_entry(
    app: &tauri::AppHandle,
    task_id: &str,
    website_url: String,
    zip_url: String,
    project_name: String,
    max_project_size_mb: u64,
) -> Result<(), String> {
    emit_catalog_import_progress(
        app.clone(),
        CatalogImportProgress {
            task_id: task_id.to_string(),
            phase: "trying-website".to_string(),
            current: 0,
            total: 100,
            message: format!("Trying website link for {}", project_name.trim()),
            done: false,
            success: false,
            error: None,
        },
    );

    let library = app.state::<LibraryState>();
    let desired_name = project_name.trim();

    let saved_project_path = match download_catalog_website_project(
        Some((app, task_id)),
        website_url.trim(),
        desired_name,
    ) {
        Ok(path) => path,
        Err(error) => {
            emit_catalog_import_progress(
                app.clone(),
                CatalogImportProgress {
                    task_id: task_id.to_string(),
                    phase: "fallback-archive".to_string(),
                    current: 20,
                    total: 100,
                    message: format!("Website link failed, falling back to ZIP: {}", error),
                    done: false,
                    success: false,
                    error: None,
                },
            );

            download_catalog_project_zip(
                zip_url.trim(),
                desired_name,
                Some((app, task_id)),
            )?
        }
    };

    emit_catalog_import_progress(
        app.clone(),
        CatalogImportProgress {
            task_id: task_id.to_string(),
            phase: "importing-project".to_string(),
            current: 90,
            total: 100,
            message: format!("Adding {} to the library", desired_name),
            done: false,
            success: false,
            error: None,
        },
    );

    let mut project = add_project_from_path(
        saved_project_path.to_string_lossy().to_string(),
        &library,
        normalize_source_url(&website_url),
    )?;

    apply_catalog_project_name_override(&mut project, desired_name, &library)?;

    let max_project_size_bytes = max_project_size_mb.saturating_mul(1024 * 1024);
    ensure_project_within_size_limit(&project.id, max_project_size_bytes, &library)?;

    emit_catalog_import_progress(
        app.clone(),
        CatalogImportProgress {
            task_id: task_id.to_string(),
            phase: "done".to_string(),
            current: 100,
            total: 100,
            message: format!("Imported {}", project.name),
            done: true,
            success: true,
            error: None,
        },
    );

    Ok(())
}

fn run_overwrite_catalog_entry(
    app: &tauri::AppHandle,
    task_id: &str,
    project_id: &str,
    website_url: String,
    zip_url: String,
    project_name: String,
    max_project_size_mb: u64,
) -> Result<(), String> {
    emit_catalog_import_progress(
        app.clone(),
        CatalogImportProgress {
            task_id: task_id.to_string(),
            phase: "trying-website".to_string(),
            current: 0,
            total: 100,
            message: format!("Trying website link for {}", project_name.trim()),
            done: false,
            success: false,
            error: None,
        },
    );

    let library = app.state::<LibraryState>();
    let desired_name = project_name.trim();
    let overwrite_destination = existing_project_file_path(project_id, &library);

    let saved_project_path = match download_catalog_website_project_to_destination(
        Some((app, task_id)),
        website_url.trim(),
        desired_name,
        overwrite_destination.as_deref(),
    ) {
        Ok(path) => path,
        Err(error) => {
            emit_catalog_import_progress(
                app.clone(),
                CatalogImportProgress {
                    task_id: task_id.to_string(),
                    phase: "fallback-archive".to_string(),
                    current: 20,
                    total: 100,
                    message: format!("Website link failed, falling back to ZIP: {}", error),
                    done: false,
                    success: false,
                    error: None,
                },
            );

            download_catalog_project_zip(
                zip_url.trim(),
                desired_name,
                Some((app, task_id)),
            )?
        }
    };

    emit_catalog_import_progress(
        app.clone(),
        CatalogImportProgress {
            task_id: task_id.to_string(),
            phase: "importing-project".to_string(),
            current: 90,
            total: 100,
            message: format!("Overwriting {} in the library", desired_name),
            done: false,
            success: false,
            error: None,
        },
    );

    let project = replace_project_from_path(
        project_id,
        saved_project_path.to_string_lossy().to_string(),
        &library,
        normalize_source_url(&website_url),
        desired_name,
    )?;

    let max_project_size_bytes = max_project_size_mb.saturating_mul(1024 * 1024);
    ensure_project_within_size_limit(&project.id, max_project_size_bytes, &library)?;

    emit_catalog_import_progress(
        app.clone(),
        CatalogImportProgress {
            task_id: task_id.to_string(),
            phase: "done".to_string(),
            current: 100,
            total: 100,
            message: format!("Overwrote {}", project.name),
            done: true,
            success: true,
            error: None,
        },
    );

    Ok(())
}

fn download_catalog_website_project(
    progress: Option<(&tauri::AppHandle, &str)>,
    website_url: &str,
    project_name: &str,
) -> Result<PathBuf, String> {
    download_catalog_website_project_to_destination(progress, website_url, project_name, None)
}

fn download_catalog_website_project_to_destination(
    progress: Option<(&tauri::AppHandle, &str)>,
    website_url: &str,
    project_name: &str,
    destination_override: Option<&Path>,
) -> Result<PathBuf, String> {
    let trimmed_url = website_url.trim();
    if trimmed_url.is_empty() {
        return Err("Missing website URL".to_string());
    }

    if let Some((app, task_id)) = progress {
        emit_catalog_import_progress(
            app.clone(),
            CatalogImportProgress {
                task_id: task_id.to_string(),
                phase: "resolving-website".to_string(),
                current: 5,
                total: 100,
                message: format!("Resolving website link for {}", project_name),
                done: false,
                success: false,
                error: None,
            },
        );
    }

    let parsed = tauri::Url::parse(trimmed_url).map_err(|e| format!("Invalid URL: {}", e))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http:// and https:// URLs are supported".to_string());
    }

    if let Some((app, task_id)) = progress {
        emit_catalog_import_progress(
            app.clone(),
            CatalogImportProgress {
                task_id: task_id.to_string(),
                phase: "downloading-project".to_string(),
                current: 20,
                total: 100,
                message: format!("Downloading project data for {}", project_name),
                done: false,
                success: false,
                error: None,
            },
        );
    }

    let (bytes, base_url) = download_project_data(parsed, progress.map(|(app, task_id)| (app, task_id, true)))?;

    if let Some((app, task_id)) = progress {
        emit_catalog_import_progress(
            app.clone(),
            CatalogImportProgress {
                task_id: task_id.to_string(),
                phase: "processing-project".to_string(),
                current: 55,
                total: 100,
                message: format!(
                    "Processing linked images for {} ({})",
                    project_name,
                    format_bytes_megabytes(bytes.len() as u64)
                ),
                done: false,
                success: false,
                error: None,
            },
        );
    }

    let processed = if let Some((app, task_id)) = progress {
        inline_downloaded_project_images(bytes.as_ref(), &base_url, app, task_id)?
    } else {
        inline_downloaded_project_images_silent(bytes.as_ref(), &base_url)?
    };

    let dir = cyoas_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create cyoas folder: {}", e))?;

    if let Some((app, task_id)) = progress {
        emit_catalog_import_progress(
            app.clone(),
            CatalogImportProgress {
                task_id: task_id.to_string(),
                phase: "saving-project".to_string(),
                current: 85,
                total: 100,
                message: format!("Saving {}", project_name),
                done: false,
                success: false,
                error: None,
            },
        );
    }

    let destination = destination_override
        .map(Path::to_path_buf)
        .unwrap_or_else(|| unique_path(dir.join(download_file_name(&base_url))));
    std::fs::write(&destination, processed).map_err(|e| format!("Failed to save file: {}", e))?;

    Ok(destination)
}

fn download_project_data(
    url: tauri::Url,
    progress: Option<(&tauri::AppHandle, &str, bool)>,
) -> Result<(Vec<u8>, tauri::Url), String> {
    if is_cyoa_cafe_host(url.host_str()) {
        return download_cyoa_cafe_project_data(url, progress);
    }

    if is_itch_io_host(url.host_str()) {
        return download_itch_io_project_data(url, progress);
    }

    if is_direct_project_json_url(&url) {
        return download_project_json_bytes(url, progress);
    }

    for project_url in candidate_project_json_urls(&url) {
        if let Ok(project) = download_project_json_bytes(project_url, progress) {
            return Ok(project);
        }
    }

    let mut last_page_error = None;

    for page_candidate in candidate_page_urls(&url) {
        let (html, final_page_url) = match fetch_html_page(page_candidate, "Failed to open website") {
            Ok(result) => result,
            Err(error) => {
                last_page_error = Some(error);
                continue;
            }
        };

        if let Some(project_url) = find_project_json_url_in_html(&html, &final_page_url) {
            if let Ok(project) = download_project_json_bytes(project_url, progress) {
                return Ok(project);
            }
        }

        if let Some((json, base_url)) = find_embedded_project_data(&html, &final_page_url)? {
            let bytes = serde_json::to_vec(&json)
                .map_err(|e| format!("Failed to serialize embedded project data: {}", e))?;
            return Ok((bytes, base_url));
        }
    }

    if let Some(error) = last_page_error {
        return Err(error);
    }

    Err("Could not find project.json or embedded project data on the provided page".to_string())
}

fn download_itch_io_project_data(
    url: tauri::Url,
    progress: Option<(&tauri::AppHandle, &str, bool)>,
) -> Result<(Vec<u8>, tauri::Url), String> {
    let (html, final_url) = fetch_html_page(url, "Failed to open itch.io page")?;

    if let Some(iframe_url) = find_itch_io_iframe_url_in_html(&html, &final_url) {
        if iframe_url != final_url {
            return download_project_data(iframe_url, progress);
        }
    }

    if let Some(project_url) = find_project_json_url_in_html(&html, &final_url) {
        if let Ok(project) = download_project_json_bytes(project_url, progress) {
            return Ok(project);
        }
    }

    if let Some((json, base_url)) = find_embedded_project_data(&html, &final_url)? {
        let bytes = serde_json::to_vec(&json)
            .map_err(|e| format!("Failed to serialize embedded project data: {}", e))?;
        return Ok((bytes, base_url));
    }

    Err("Could not find an embedded CYOA iframe, project.json link, or embedded project data on the provided itch.io page".to_string())
}

fn download_cyoa_cafe_project_data(
    url: tauri::Url,
    progress: Option<(&tauri::AppHandle, &str, bool)>,
) -> Result<(Vec<u8>, tauri::Url), String> {
    if let Some(game_id) = extract_cyoa_cafe_game_id(&url) {
        if let Some(source_url) = fetch_cyoa_cafe_game_source_url(&game_id)? {
            let parsed_source = tauri::Url::parse(source_url.trim())
                .map_err(|e| format!("Invalid source URL from cyoa.cafe: {}", e))?;
            return download_project_data(parsed_source, progress);
        }
    }

    let (html, final_url) = fetch_html_page(url, "Failed to open cyoa.cafe page")?;
    if let Some(project_url) = find_project_json_url_in_html(&html, &final_url) {
        if let Ok(project) = download_project_json_bytes(project_url, progress) {
            return Ok(project);
        }
    }

    if let Some((json, base_url)) = find_embedded_project_data(&html, &final_url)? {
        let bytes = serde_json::to_vec(&json)
            .map_err(|e| format!("Failed to serialize embedded project data: {}", e))?;
        return Ok((bytes, base_url));
    }

    Err("Could not find a project.json link or embedded project data on the provided cyoa.cafe page".to_string())
}

fn download_project_json_bytes(
    url: tauri::Url,
    progress: Option<(&tauri::AppHandle, &str, bool)>,
) -> Result<(Vec<u8>, tauri::Url), String> {
    let response = reqwest::blocking::get(url.clone())
        .map_err(|e| format!("Download failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {}", e))?;

    let final_url = tauri::Url::parse(response.url().as_str())
        .map_err(|e| format!("Failed to parse downloaded URL: {}", e))?;
    let bytes = read_response_with_progress(response, |downloaded, total| {
        if let Some((app, task_id, is_catalog)) = progress {
            emit_transfer_progress(app.clone(), task_id, is_catalog, downloaded, total);
        }
    })
    .map_err(|e| format!("Failed to read response body: {}", e))?;

    serde_json::from_slice::<serde_json::Value>(bytes.as_ref())
        .map_err(|e| format!("Downloaded file was not valid JSON: {}", e))?;

    Ok((bytes, final_url))
}

fn read_response_with_progress<F>(
    mut response: reqwest::blocking::Response,
    mut on_progress: F,
) -> Result<Vec<u8>, std::io::Error>
where
    F: FnMut(u64, Option<u64>),
{
    let total = response.content_length();
    let mut downloaded: u64 = 0;
    let mut output = Vec::new();
    let mut chunk = [0u8; 64 * 1024];

    on_progress(0, total);
    loop {
        let read = response.read(&mut chunk)?;
        if read == 0 {
            break;
        }

        output.extend_from_slice(&chunk[..read]);
        downloaded = downloaded.saturating_add(read as u64);
        on_progress(downloaded, total);
    }

    Ok(output)
}

fn emit_transfer_progress(
    app: tauri::AppHandle,
    task_id: &str,
    is_catalog: bool,
    downloaded: u64,
    total: Option<u64>,
) {
    let (current, total_units) = match total {
        Some(t) if t > 0 => (
            usize::try_from(downloaded.min(t)).unwrap_or(usize::MAX),
            usize::try_from(t).unwrap_or(usize::MAX),
        ),
        _ => {
            let fallback_total = downloaded.saturating_add(1);
            (
                usize::try_from(downloaded).unwrap_or(usize::MAX),
                usize::try_from(fallback_total).unwrap_or(usize::MAX),
            )
        }
    };

    let message = match total {
        Some(t) if t > 0 => format!(
            "Downloading project payload: {} / {}",
            format_bytes_megabytes(downloaded),
            format_bytes_megabytes(t),
        ),
        _ => format!(
            "Downloading project payload: {}",
            format_bytes_megabytes(downloaded),
        ),
    };

    if is_catalog {
        emit_catalog_import_progress(
            app,
            CatalogImportProgress {
                task_id: task_id.to_string(),
                phase: "downloading-project".to_string(),
                current,
                total: total_units,
                message,
                done: false,
                success: false,
                error: None,
            },
        );
    } else {
        emit_download_progress(
            app,
            DownloadProjectProgress {
                task_id: task_id.to_string(),
                phase: "fetching-project".to_string(),
                current,
                total: total_units,
                image_current: 0,
                image_total: 0,
                message,
                done: false,
                success: false,
                error: None,
            },
        );
    }
}

fn fetch_html_page(url: tauri::Url, error_prefix: &str) -> Result<(String, tauri::Url), String> {
    let response = reqwest::blocking::get(url)
        .map_err(|e| format!("{}: {}", error_prefix, e))?
        .error_for_status()
        .map_err(|e| format!("{}: {}", error_prefix, e))?;
    let final_url = tauri::Url::parse(response.url().as_str())
        .map_err(|e| format!("Failed to parse page URL: {}", e))?;
    let html = response
        .text()
        .map_err(|e| format!("Failed to read page HTML: {}", e))?;

    Ok((html, final_url))
}

fn maybe_download_included_icc_plus_viewer(
    app: &tauri::AppHandle,
    task_id: &str,
    base_url: &tauri::Url,
    project_bytes: &[u8],
) -> Result<Option<String>, String> {
    emit_download_progress(
        app.clone(),
        DownloadProjectProgress {
            task_id: task_id.to_string(),
            phase: "checking-viewer".to_string(),
            current: 0,
            total: 1,
            image_current: 0,
            image_total: 0,
            message: "Checking for included ICC+ viewer".to_string(),
            done: false,
            success: false,
            error: None,
        },
    );

    let viewer_root_url = build_viewer_root_url(base_url)?;
    let index_url = viewer_root_url
        .join("index.html")
        .map_err(|error| format!("Failed to resolve ICC+ viewer index: {}", error))?;
    let (index_html, index_url) = match fetch_html_page(index_url, "Failed to download ICC+ viewer index") {
        Ok(result) => result,
        Err(_) => return Ok(None),
    };

    let mut assets = HashMap::new();
    assets.insert("index.html".to_string(), index_url.clone());
    for raw_path in collect_html_asset_paths(&index_html) {
        add_viewer_asset(&mut assets, &viewer_root_url, &index_url, &raw_path);
    }

    let core_relative_path = assets
        .keys()
        .find(|path| path.ends_with("core.js"))
        .cloned();
    let app_relative_path = assets
        .keys()
        .find(|path| path.ends_with("app.js"))
        .cloned();

    if core_relative_path.is_none() && app_relative_path.is_none() {
        emit_download_progress(
            app.clone(),
            DownloadProjectProgress {
                task_id: task_id.to_string(),
                phase: "checking-viewer".to_string(),
                current: 1,
                total: 1,
                image_current: 0,
                image_total: 0,
                message: "No bundled ICC+ viewer detected on the site".to_string(),
                done: false,
                success: false,
                error: None,
            },
        );
        return Ok(None);
    }

    if let Some(core_relative_path) = core_relative_path {
        let core_url = assets
            .get(&core_relative_path)
            .cloned()
            .ok_or_else(|| "Failed to resolve ICC+ core.js URL".to_string())?;
        let core_js = download_text_asset(core_url.clone(), "Failed to download ICC+ core.js")?;

        for raw_path in collect_core_js_asset_paths(&core_js) {
            add_viewer_asset(&mut assets, &viewer_root_url, &viewer_root_url, &raw_path);
        }
    }

    let css_assets: Vec<(String, tauri::Url)> = assets
        .iter()
        .filter(|(path, _)| path.ends_with(".css"))
        .map(|(path, url)| (path.clone(), url.clone()))
        .collect();

    for (_, css_url) in css_assets {
        let css = download_text_asset(css_url.clone(), "Failed to download ICC+ stylesheet")?;
        for raw_path in collect_css_asset_paths(&css) {
            add_viewer_asset(&mut assets, &viewer_root_url, &css_url, &raw_path);
        }
    }

    let version = extract_project_version(project_bytes)
        .map(|value| sanitize_version_label(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string()[..8].to_string());
    let viewer_name = format!("ICC2 Plus {}", version);
    let viewer_id = slugify(&viewer_name);
    let viewer_dir = viewers_base_dir(Some(app)).join(&viewer_name);

    std::fs::create_dir_all(&viewer_dir)
        .map_err(|error| format!("Failed to create viewer folder: {}", error))?;

    emit_download_progress(
        app.clone(),
        DownloadProjectProgress {
            task_id: task_id.to_string(),
            phase: "downloading-viewer".to_string(),
            current: 0,
            total: assets.len().max(1),
            image_current: 0,
            image_total: 0,
            message: format!("Downloading bundled ICC+ viewer {}", version),
            done: false,
            success: false,
            error: None,
        },
    );

    let total_assets = assets.len().max(1);
    let mut completed = 0usize;
    let mut ordered_assets: Vec<_> = assets.into_iter().collect();
    ordered_assets.sort_by(|left, right| left.0.cmp(&right.0));

    for (relative_path, asset_url) in ordered_assets {
        let bytes = download_binary_asset(asset_url, "Failed to download ICC+ asset")?;
        let destination = viewer_dir.join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create ICC+ asset folder: {}", error))?;
        }
        std::fs::write(&destination, bytes)
            .map_err(|error| format!("Failed to save ICC+ asset: {}", error))?;

        completed += 1;
        emit_download_progress(
            app.clone(),
            DownloadProjectProgress {
                task_id: task_id.to_string(),
                phase: "downloading-viewer".to_string(),
                current: completed,
                total: total_assets,
                image_current: 0,
                image_total: 0,
                message: format!("Downloading bundled ICC+ viewer {} ({}/{})", version, completed, total_assets),
                done: false,
                success: false,
                error: None,
            },
        );
    }

    Ok(Some(viewer_id))
}

fn build_viewer_root_url(base_url: &tauri::Url) -> Result<tauri::Url, String> {
    let mut root = base_url.clone();
    root.set_query(None);
    root.set_fragment(None);

    if !root.path().ends_with('/') {
        let mut segments = root
            .path_segments_mut()
            .map_err(|_| "Failed to resolve viewer root URL".to_string())?;
        segments.pop_if_empty();
        segments.pop();
    }

    if root.path().is_empty() {
        root.set_path("/");
    } else if !root.path().ends_with('/') {
        root.set_path(&format!("{}/", root.path()));
    }

    Ok(root)
}

fn add_viewer_asset(
    assets: &mut HashMap<String, tauri::Url>,
    root_url: &tauri::Url,
    base_url: &tauri::Url,
    raw_path: &str,
) {
    let trimmed = raw_path.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("data:")
        || trimmed.starts_with("javascript:")
        || trimmed.starts_with('#')
    {
        return;
    }

    let Ok(asset_url) = base_url.join(trimmed) else {
        return;
    };

    let Some(relative_path) = viewer_asset_relative_path(root_url, &asset_url) else {
        return;
    };

    assets.entry(relative_path).or_insert(asset_url);
}

fn viewer_asset_relative_path(root_url: &tauri::Url, asset_url: &tauri::Url) -> Option<String> {
    if root_url.scheme() != asset_url.scheme()
        || root_url.host_str() != asset_url.host_str()
        || root_url.port_or_known_default() != asset_url.port_or_known_default()
    {
        return None;
    }

    let root_path = root_url.path();
    let asset_path = asset_url.path();
    if !asset_path.starts_with(root_path) {
        return None;
    }

    let relative = asset_path[root_path.len()..].trim_start_matches('/');
    if relative.is_empty() {
        return None;
    }

    let mut clean_segments = Vec::new();
    for segment in relative.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return None;
        }
        clean_segments.push(segment);
    }

    if clean_segments.is_empty() {
        return None;
    }

    Some(clean_segments.join("/"))
}

fn collect_html_asset_paths(html: &str) -> Vec<String> {
    let pattern = Regex::new(r#"(?:src|href)\s*=\s*["']([^"']+)["']"#)
        .expect("valid html asset regex");
    pattern
        .captures_iter(html)
        .filter_map(|captures| captures.get(1).map(|value| value.as_str().to_string()))
        .collect()
}

fn collect_core_js_asset_paths(core_js: &str) -> Vec<String> {
    let pattern = Regex::new(r#"basePath\s*\+\s*["']([^"']+)["']"#)
        .expect("valid core js asset regex");
    pattern
        .captures_iter(core_js)
        .filter_map(|captures| captures.get(1).map(|value| value.as_str().to_string()))
        .collect()
}

fn collect_css_asset_paths(css: &str) -> Vec<String> {
    let pattern = Regex::new(r#"url\(\s*['\"]?([^)'\"]+)['\"]?\s*\)"#)
        .expect("valid css asset regex");
    pattern
        .captures_iter(css)
        .filter_map(|captures| captures.get(1).map(|value| value.as_str().to_string()))
        .collect()
}

fn download_text_asset(url: tauri::Url, error_prefix: &str) -> Result<String, String> {
    let response = reqwest::blocking::get(url)
        .map_err(|error| format!("{}: {}", error_prefix, error))?
        .error_for_status()
        .map_err(|error| format!("{}: {}", error_prefix, error))?;
    response
        .text()
        .map_err(|error| format!("{}: {}", error_prefix, error))
}

fn download_binary_asset(url: tauri::Url, error_prefix: &str) -> Result<Vec<u8>, String> {
    let response = reqwest::blocking::get(url)
        .map_err(|error| format!("{}: {}", error_prefix, error))?
        .error_for_status()
        .map_err(|error| format!("{}: {}", error_prefix, error))?;
    response
        .bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|error| format!("{}: {}", error_prefix, error))
}

fn extract_project_version(project_bytes: &[u8]) -> Option<String> {
    let json = serde_json::from_slice::<serde_json::Value>(project_bytes).ok()?;
    json.get("version")?.as_str().map(|value| value.trim().to_string())
}

fn sanitize_version_label(version: &str) -> String {
    version
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn candidate_page_urls(url: &tauri::Url) -> Vec<tauri::Url> {
    let mut urls = vec![url.clone()];

    if let Some(html_url) = build_extensionless_html_variant_url(url) {
        if html_url != *url {
            urls.push(html_url);
        }
    }

    urls
}

fn candidate_project_json_urls(url: &tauri::Url) -> Vec<tauri::Url> {
    let mut urls = Vec::new();
    let mut seen = HashSet::new();

    let default_url = build_default_project_json_url(url);
    if seen.insert(default_url.to_string()) {
        urls.push(default_url);
    }

    if let Some(html_url) = build_extensionless_html_variant_url(url) {
        let html_default_url = build_default_project_json_url(&html_url);
        if seen.insert(html_default_url.to_string()) {
            urls.push(html_default_url);
        }
    }

    urls
}

fn build_extensionless_html_variant_url(url: &tauri::Url) -> Option<tauri::Url> {
    let mut html_url = url.clone();
    html_url.set_query(None);
    html_url.set_fragment(None);

    let path = html_url.path();
    if path.is_empty() || path == "/" || path.ends_with('/') {
        return None;
    }

    let last_segment = path.rsplit('/').next().unwrap_or_default();
    if last_segment.is_empty() || last_segment.contains('.') {
        return None;
    }

    html_url.set_path(&format!("{}.html", path));
    Some(html_url)
}

fn find_embedded_project_data(
    html: &str,
    page_url: &tauri::Url,
) -> Result<Option<(serde_json::Value, tauri::Url)>, String> {
    for script in extract_inline_script_blocks(html) {
        if let Some(project) = extract_embedded_project_json(script) {
            return Ok(Some((project, build_default_project_json_url(page_url))));
        }
    }

    for script_url in find_hashed_app_script_urls_in_html(html, page_url) {
        let response = match reqwest::blocking::get(script_url.clone()) {
            Ok(response) => response,
            Err(_) => continue,
        };
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(_) => continue,
        };
        let script = match response.text() {
            Ok(script) => script,
            Err(_) => continue,
        };

        if let Some(project) = extract_embedded_project_json(&script) {
            return Ok(Some((project, build_default_project_json_url(page_url))));
        }
    }

    for script_url in find_script_urls_in_html(html, page_url) {
        let response = match reqwest::blocking::get(script_url.clone()) {
            Ok(response) => response,
            Err(_) => continue,
        };
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(_) => continue,
        };
        let script = match response.text() {
            Ok(script) => script,
            Err(_) => continue,
        };

        if let Some(project) = extract_embedded_project_json(&script) {
            return Ok(Some((project, build_default_project_json_url(page_url))));
        }
    }

    Ok(None)
}

fn apply_catalog_project_name_override(
    project: &mut Project,
    desired_name: &str,
    state: &State<LibraryState>,
) -> Result<(), String> {
    if project.name == desired_name || desired_name.is_empty() {
        return Ok(());
    }

    let mut lib = state.lock().map_err(|e| e.to_string())?;
    let index = lib
        .projects
        .iter()
        .position(|candidate| candidate.id == project.id)
        .ok_or_else(|| format!("Project not found after import: {}", project.id))?;

    let mut updated = lib.projects[index].clone();
    updated.name = desired_name.to_string();
    persist_project(&updated)?;

    lib.projects[index] = updated.clone();
    project.name = updated.name;
    Ok(())
}

fn set_project_viewer_preference_after_import(
    project_id: &str,
    viewer_id: &str,
    state: &State<LibraryState>,
) -> Result<Project, String> {
    persist_project_viewer_preference(project_id, Some(viewer_id))?;

    let mut lib = state.lock().map_err(|e| e.to_string())?;
    let index = lib
        .projects
        .iter()
        .position(|candidate| candidate.id == project_id)
        .ok_or_else(|| format!("Project not found after import: {}", project_id))?;

    let mut updated = lib.projects[index].clone();
    updated.viewer_preference = Some(viewer_id.to_string());
    lib.projects[index] = updated.clone();
    Ok(updated)
}

fn download_catalog_project_zip(
    zip_url: &str,
    project_name: &str,
    progress: Option<(&tauri::AppHandle, &str)>,
) -> Result<PathBuf, String> {
    if zip_url.is_empty() {
        return Err("Missing ZIP URL".to_string());
    }

    let response = reqwest::blocking::get(zip_url)
        .map_err(|e| format!("Download failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {}", e))?;
    let bytes = read_response_with_progress(response, |downloaded, total| {
        if let Some((app, task_id)) = progress {
            let (current, total_units) = match total {
                Some(t) if t > 0 => (
                    usize::try_from(downloaded.min(t)).unwrap_or(usize::MAX),
                    usize::try_from(t).unwrap_or(usize::MAX),
                ),
                _ => {
                    let fallback_total = downloaded.saturating_add(1);
                    (
                        usize::try_from(downloaded).unwrap_or(usize::MAX),
                        usize::try_from(fallback_total).unwrap_or(usize::MAX),
                    )
                }
            };

            let message = match total {
                Some(t) if t > 0 => format!(
                    "Downloading ZIP: {} / {}",
                    format_bytes_megabytes(downloaded),
                    format_bytes_megabytes(t),
                ),
                _ => format!("Downloading ZIP: {}", format_bytes_megabytes(downloaded)),
            };

            emit_catalog_import_progress(
                app.clone(),
                CatalogImportProgress {
                    task_id: task_id.to_string(),
                    phase: "downloading-archive".to_string(),
                    current,
                    total: total_units,
                    message,
                    done: false,
                    success: false,
                    error: None,
                },
            );
        }
    })
    .map_err(|e| format!("Failed to read ZIP archive: {}", e))?;

    if let Some((app, task_id)) = progress {
        emit_catalog_import_progress(
            app.clone(),
            CatalogImportProgress {
                task_id: task_id.to_string(),
                phase: "extracting-archive".to_string(),
                current: 25,
                total: 100,
                message: format!(
                    "Extracting {} ({})",
                    project_name,
                    format_bytes_megabytes(bytes.len() as u64)
                ),
                done: false,
                success: false,
                error: None,
            },
        );
    }

    let archive_name = if slugify(project_name).is_empty() {
        "downloaded-cyoa".to_string()
    } else {
        slugify(project_name)
    };

    let extraction_root = unique_dir_path(cyoas_dir().join(&archive_name));
    std::fs::create_dir_all(&extraction_root)
        .map_err(|e| format!("Failed to create extraction folder: {}", e))?;

    if let Err(error) = extract_catalog_zip(&bytes, &extraction_root, progress) {
        let _ = std::fs::remove_dir_all(&extraction_root);
        return Err(error);
    }

    if let Some((app, task_id)) = progress {
        emit_catalog_import_progress(
            app.clone(),
            CatalogImportProgress {
                task_id: task_id.to_string(),
                phase: "scanning-archive".to_string(),
                current: 75,
                total: 100,
                message: format!("Finding project JSON for {}", project_name),
                done: false,
                success: false,
                error: None,
            },
        );
    }

    let Some(project_path) = find_best_project_json_path(&extraction_root) else {
        let _ = std::fs::remove_dir_all(&extraction_root);
        return Err("No usable project JSON file was found in the ZIP archive".to_string());
    };

    Ok(project_path)
}

fn extract_catalog_zip(
    bytes: &[u8],
    destination: &Path,
    progress: Option<(&tauri::AppHandle, &str)>,
) -> Result<(), String> {
    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("Downloaded file is not a valid ZIP archive: {}", e))?;
    let total_entries = archive.len().max(1);

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Failed to read ZIP entry: {}", e))?;
        let Some(relative_path) = entry.enclosed_name().map(|path| path.to_path_buf()) else {
            continue;
        };

        let output_path = destination.join(relative_path);
        if entry.is_dir() {
            std::fs::create_dir_all(&output_path)
                .map_err(|e| format!("Failed to create extracted folder: {}", e))?;
        } else {
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create extracted folder: {}", e))?;
            }

            let mut file = std::fs::File::create(&output_path)
                .map_err(|e| format!("Failed to create extracted file: {}", e))?;
            std::io::copy(&mut entry, &mut file)
                .map_err(|e| format!("Failed to extract ZIP entry: {}", e))?;
            file.flush()
                .map_err(|e| format!("Failed to finalize extracted file: {}", e))?;
        }

        if let Some((app, task_id)) = progress {
            let percent = 25 + ((index + 1) * 50 / total_entries);
            emit_catalog_import_progress(
                app.clone(),
                CatalogImportProgress {
                    task_id: task_id.to_string(),
                    phase: "extracting-archive".to_string(),
                    current: percent,
                    total: 100,
                    message: format!("Extracting archive files {}/{}", index + 1, total_entries),
                    done: false,
                    success: false,
                    error: None,
                },
            );
        }
    }

    Ok(())
}

fn find_best_project_json_path(root: &Path) -> Option<PathBuf> {
    use walkdir::WalkDir;

    let mut candidates = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(|entry| entry.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        let score = score_project_json_candidate(path, &json);
        if score > 0 {
            candidates.push((score, path.to_path_buf()));
        }
    }

    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

fn score_project_json_candidate(path: &Path, json: &serde_json::Value) -> i32 {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return 0;
    };

    let lower_name = file_name.to_ascii_lowercase();
    let mut score = 0;

    if lower_name == "project.json" {
        score += 100;
    }
    if lower_name.contains("project") {
        score += 40;
    }
    if json.get("rows").and_then(|value| value.as_array()).is_some() {
        score += 40;
    }
    if json.get("styling").map(|value| value.is_object()).unwrap_or(false) {
        score += 25;
    }
    if extract_project_name(json).is_some() {
        score += 15;
    }
    if extract_first_row_title(json).is_some() {
        score += 10;
    }

    score
}

fn build_default_project_json_url(url: &tauri::Url) -> tauri::Url {
    let mut project_url = url.clone();
    project_url.set_query(None);
    project_url.set_fragment(None);

    let path = project_url.path();
    let new_path = if path.is_empty() || path == "/" {
        "/project.json".to_string()
    } else if path.ends_with('/') {
        format!("{}project.json", path)
    } else {
        let last_segment = path.rsplit('/').next().unwrap_or_default();
        if last_segment.contains('.') {
            match path.rsplit_once('/') {
                Some((prefix, _)) if !prefix.is_empty() => format!("{}/project.json", prefix),
                _ => "/project.json".to_string(),
            }
        } else {
            format!("{}/project.json", path)
        }
    };

    project_url.set_path(&new_path);
    project_url
}

fn is_cyoa_cafe_host(host: Option<&str>) -> bool {
    host.map(|value| {
        let lower = value.to_ascii_lowercase();
        lower == "cyoa.cafe" || lower.ends_with(".cyoa.cafe")
    })
    .unwrap_or(false)
}

fn is_itch_io_host(host: Option<&str>) -> bool {
    host.map(|value| {
        let lower = value.to_ascii_lowercase();
        lower == "itch.io" || lower.ends_with(".itch.io")
    })
    .unwrap_or(false)
}

fn extract_cyoa_cafe_game_id(url: &tauri::Url) -> Option<String> {
    let mut segments = url.path_segments()?;
    let first = segments.next()?;
    if !first.eq_ignore_ascii_case("game") {
        return None;
    }

    let game_id = segments.next()?.trim();
    if game_id.is_empty() {
        return None;
    }

    Some(game_id.to_string())
}

fn fetch_cyoa_cafe_game_source_url(game_id: &str) -> Result<Option<String>, String> {
    let api_url = format!(
        "https://cyoa.cafe/api/collections/games/records/{}?fields=iframe_url,img_or_link",
        game_id
    );

    let response = reqwest::blocking::get(&api_url)
        .map_err(|e| format!("Failed to query cyoa.cafe game API: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Failed to query cyoa.cafe game API: {}", e))?;

    let body = response
        .text()
        .map_err(|e| format!("Failed to read cyoa.cafe game API response: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse cyoa.cafe game API response: {}", e))?;

    let source_url = json
        .get("iframe_url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    if source_url.is_none() {
        let mode = json
            .get("img_or_link")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if mode != "link" {
            return Err("This cyoa.cafe entry does not provide a downloadable project link".to_string());
        }
    }

    Ok(source_url)
}

fn find_project_json_url_in_html(html: &str, page_url: &tauri::Url) -> Option<tauri::Url> {
    let needle = "project.json";
    let lower_html = html.to_ascii_lowercase();
    let mut cursor = 0;
    let mut seen = HashSet::new();

    while let Some(found_at) = lower_html[cursor..].find(needle) {
        let index = cursor + found_at;
        if let Some(token) = extract_url_token_around(html, index, needle.len()) {
            let cleaned = clean_url_token(&token);
            if !cleaned.is_empty() && seen.insert(cleaned.clone()) {
                if let Some(resolved) = parse_project_json_candidate(&cleaned, page_url) {
                    return Some(resolved);
                }
            }
        }
        cursor = index + needle.len();
    }

    None
}

fn find_itch_io_iframe_url_in_html(html: &str, page_url: &tauri::Url) -> Option<tauri::Url> {
    let lower_html = html.to_ascii_lowercase();
    let mut cursor = 0;
    let mut fallback = None;

    while let Some(found_at) = lower_html[cursor..].find("<") {
        let tag_start = cursor + found_at;
        let Some(tag_end_relative) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + tag_end_relative + 1;
        let tag = &html[tag_start..tag_end];

        if let Some(url) = extract_itch_io_embed_url(tag, page_url) {
            if points_to_index_html(&url) {
                return Some(url);
            }

            if fallback.is_none() {
                fallback = Some(url);
            }
        }

        cursor = tag_end;
    }

    fallback
}

fn extract_itch_io_embed_url(tag: &str, page_url: &tauri::Url) -> Option<tauri::Url> {
    if let Some(src) = extract_html_attribute(tag, "src") {
        if let Some(url) = parse_itch_io_embed_candidate(&src, page_url) {
            return Some(url);
        }
    }

    let encoded_iframe = extract_html_attribute(tag, "data-iframe")?;
    let decoded_iframe = decode_basic_html_entities(&encoded_iframe);
    let src = extract_html_attribute(&decoded_iframe, "src")?;
    parse_itch_io_embed_candidate(&src, page_url)
}

fn parse_itch_io_embed_candidate(raw: &str, page_url: &tauri::Url) -> Option<tauri::Url> {
    let normalized = strip_wrapping_url_quotes(
        decode_basic_html_entities(raw)
            .replace("\\/", "/")
            .trim(),
    );

    let url = if normalized.starts_with("http://") || normalized.starts_with("https://") {
        tauri::Url::parse(&normalized).ok()?
    } else if normalized.starts_with("//") {
        tauri::Url::parse(&format!("{}:{}", page_url.scheme(), normalized)).ok()?
    } else {
        page_url.join(&normalized).ok()?
    };

    if matches!(url.scheme(), "http" | "https") {
        Some(url)
    } else {
        None
    }
}

fn strip_wrapping_url_quotes(value: &str) -> String {
    let mut normalized = value.trim().to_string();

    loop {
        let trimmed = normalized
            .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'))
            .to_string();
        let trimmed = trimmed
            .strip_prefix("%22")
            .unwrap_or(&trimmed)
            .strip_suffix("%22")
            .unwrap_or(&trimmed)
            .strip_prefix("%27")
            .unwrap_or(&trimmed)
            .strip_suffix("%27")
            .unwrap_or(&trimmed)
            .strip_prefix("%60")
            .unwrap_or(&trimmed)
            .strip_suffix("%60")
            .unwrap_or(&trimmed)
            .to_string();

        if trimmed == normalized {
            return trimmed;
        }

        normalized = trimmed;
    }
}

fn decode_basic_html_entities(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#x22;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn points_to_index_html(url: &tauri::Url) -> bool {
    let without_suffix = url
        .as_str()
        .split(['?', '#'])
        .next()
        .unwrap_or(url.as_str())
        .trim_end_matches('/');
    let lower = without_suffix.to_ascii_lowercase();

    lower.ends_with("/index.html") || lower == "index.html"
}

fn find_script_urls_in_html(html: &str, page_url: &tauri::Url) -> Vec<tauri::Url> {
    let lower_html = html.to_ascii_lowercase();
    let mut cursor = 0;
    let mut urls = Vec::new();
    let mut seen = HashSet::new();

    while let Some(found_at) = lower_html[cursor..].find("<script") {
        let tag_start = cursor + found_at;
        let Some(tag_end_relative) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + tag_end_relative + 1;
        let tag = &html[tag_start..tag_end];

        if let Some(src) = extract_html_attribute(tag, "src") {
            if let Ok(url) = page_url.join(src.trim()) {
                let key = url.as_str().to_ascii_lowercase();
                if seen.insert(key) {
                    urls.push(url);
                }
            }
        }

        cursor = tag_end;
    }

    urls.sort_by_key(script_url_priority);
    urls
}

fn find_hashed_app_script_urls_in_html(html: &str, page_url: &tauri::Url) -> Vec<tauri::Url> {
    let script_src_regex = Regex::new(
        r#"(?i)["'`](?P<src>(?:(?:https?:)?//|/|\.{1,2}/)?[^"'`\s]*?(?:js/)?app\.[A-Za-z0-9_-]+\.js(?:\?[^"'`\s]*)?)["'`]"#,
    )
    .expect("valid app script regex");

    let mut urls = Vec::new();
    let mut seen = HashSet::new();

    for captures in script_src_regex.captures_iter(html) {
        let Some(src) = captures.name("src") else {
            continue;
        };

        let decoded = decode_basic_html_entities(src.as_str()).replace("\\/", "/");
        let normalized = strip_wrapping_url_quotes(decoded.trim());
        let Some(url) = resolve_html_url_candidate(&normalized, page_url) else {
            continue;
        };

        let key = url.as_str().to_ascii_lowercase();
        if seen.insert(key) {
            urls.push(url);
        }
    }

    urls
}

fn resolve_html_url_candidate(candidate: &str, page_url: &tauri::Url) -> Option<tauri::Url> {
    if candidate.starts_with("http://") || candidate.starts_with("https://") {
        tauri::Url::parse(candidate).ok()
    } else if candidate.starts_with("//") {
        tauri::Url::parse(&format!("{}:{}", page_url.scheme(), candidate)).ok()
    } else {
        page_url.join(candidate).ok()
    }
}

fn script_url_priority(url: &tauri::Url) -> i32 {
    let path = url.path().to_ascii_lowercase();
    let mut score = 0;

    if path.contains("app") || path.contains("main") {
        score -= 20;
    }
    if path.contains("vendor") || path.contains("chunk") || path.contains("polyfill") {
        score += 20;
    }

    score
}

fn extract_inline_script_blocks(html: &str) -> Vec<&str> {
    let lower_html = html.to_ascii_lowercase();
    let mut scripts = Vec::new();
    let mut cursor = 0;

    while let Some(found_at) = lower_html[cursor..].find("<script") {
        let tag_start = cursor + found_at;
        let Some(tag_end_relative) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + tag_end_relative + 1;
        let tag = &html[tag_start..tag_end];

        if extract_html_attribute(tag, "src").is_none() {
            if let Some(close_relative) = lower_html[tag_end..].find("</script>") {
                let close_start = tag_end + close_relative;
                scripts.push(&html[tag_end..close_start]);
                cursor = close_start + "</script>".len();
                continue;
            }
        }

        cursor = tag_end;
    }

    scripts
}

fn extract_html_attribute(tag: &str, attribute: &str) -> Option<String> {
    let lower_tag = tag.to_ascii_lowercase();
    let needle = format!("{}=", attribute.to_ascii_lowercase());
    let start = lower_tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let trimmed = rest.trim_start();

    if let Some(value) = trimmed.strip_prefix('"') {
        return value.split('"').next().map(|part| part.to_string());
    }
    if let Some(value) = trimmed.strip_prefix('\'') {
        return value.split('\'').next().map(|part| part.to_string());
    }

    let end = trimmed
        .find(|c: char| c.is_ascii_whitespace() || c == '>')
        .unwrap_or(trimmed.len());
    Some(trimmed[..end].to_string())
}

fn extract_embedded_project_json(script: &str) -> Option<serde_json::Value> {
    let mut best_match: Option<(i32, serde_json::Value)> = None;

    for marker in ["\"rows\":[", "\"rows\": [", "\"styling\":{", "\"styling\": {"] {
        let mut search_offset = 0;

        while let Some(found_at) = script[search_offset..].find(marker) {
            let marker_index = search_offset + found_at;
            let mut backtrack_end = marker_index;
            let mut attempts = 0;

            while attempts < 256 {
                let Some(open_index) = script[..backtrack_end].rfind('{') else {
                    break;
                };

                if let Some(close_index) = find_balanced_json_object_end(script, open_index) {
                    if close_index > marker_index {
                        let candidate = &script[open_index..=close_index];
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(candidate) {
                            let score = score_embedded_project_candidate(&json);
                            if score > 0 {
                                match &best_match {
                                    Some((best_score, _)) if *best_score >= score => {}
                                    _ => best_match = Some((score, json)),
                                }
                            }
                        }
                    }
                }

                backtrack_end = open_index;
                attempts += 1;
            }

            search_offset = marker_index + marker.len();
        }
    }

    best_match.map(|(_, json)| json)
}

fn find_balanced_json_object_end(source: &str, start_index: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;

    for (offset, ch) in source[start_index..].char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }

            match ch {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start_index + offset);
                }
            }
            _ => {}
        }
    }

    None
}

fn score_embedded_project_candidate(json: &serde_json::Value) -> i32 {
    let mut score = 0;

    if json.get("rows").and_then(|value| value.as_array()).is_some() {
        score += 60;
    }
    if json.get("styling").map(|value| value.is_object()).unwrap_or(false) {
        score += 35;
    }
    if json.get("version").and_then(|value| value.as_str()).is_some() {
        score += 20;
    }
    if extract_project_name(json).is_some() {
        score += 15;
    }
    if extract_first_row_title(json).is_some() {
        score += 10;
    }

    score
}

fn extract_url_token_around(html: &str, center: usize, needle_len: usize) -> Option<String> {
    let bytes = html.as_bytes();
    if center >= bytes.len() {
        return None;
    }

    let mut left = center;
    while left > 0 && !is_url_boundary(bytes[left - 1]) {
        left -= 1;
    }

    let mut right = (center + needle_len).min(bytes.len());
    while right < bytes.len() && !is_url_boundary(bytes[right]) {
        right += 1;
    }

    html.get(left..right).map(|s| s.to_string())
}

fn is_url_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace() || b"\"'<>()[]{};,".contains(&byte)
}

fn clean_url_token(token: &str) -> String {
    token
        .trim_matches(|c: char| "\"'`()<>{}[];,".contains(c))
        .replace("\\/", "/")
        .replace("&amp;", "&")
}

fn parse_project_json_candidate(candidate: &str, page_url: &tauri::Url) -> Option<tauri::Url> {
    let parsed = if candidate.starts_with("http://") || candidate.starts_with("https://") {
        tauri::Url::parse(candidate).ok()?
    } else if candidate.starts_with("//") {
        tauri::Url::parse(&format!("{}:{}", page_url.scheme(), candidate)).ok()?
    } else {
        page_url.join(candidate).ok()?
    };

    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    if is_direct_project_json_url(&parsed) {
        Some(parsed)
    } else {
        None
    }
}

fn is_direct_project_json_url(url: &tauri::Url) -> bool {
    url
        .path_segments()
        .and_then(|segments| segments.last())
        .map(|segment| segment.to_ascii_lowercase().ends_with(".json"))
        .unwrap_or(false)
}

fn detect_default_viewer_preference(json: &serde_json::Value) -> Option<String> {
    if is_icc_plus_project(json) {
        Some(slugify("ICC2 Plus"))
    } else {
        None
    }
}

fn is_icc_plus_project(json: &serde_json::Value) -> bool {
    let Some(root) = json.as_object() else {
        return false;
    };

    let Some(version) = root.get("version").and_then(|value| value.as_str()) else {
        return false;
    };

    looks_like_icc_plus_version(version)
        && root.get("rows").and_then(|value| value.as_array()).is_some()
        && root.get("styling").map(|value| value.is_object()).unwrap_or(false)
}

fn looks_like_icc_plus_version(version: &str) -> bool {
    let mut segments = version.split('.');
    let first = segments.next().filter(|segment| !segment.is_empty());
    let second = segments.next().filter(|segment| !segment.is_empty());

    if first.is_none() || second.is_none() {
        return false;
    }

    version
        .split('.')
        .all(|segment| !segment.is_empty() && segment.chars().all(|c| c.is_ascii_digit()))
}

fn add_project_from_path(
    file_path: String,
    state: &State<LibraryState>,
    source_url: Option<String>,
) -> Result<Project, String> {
    let path = Path::new(&file_path);
    if !path.exists() {
        return Err(format!("File does not exist: {}", file_path));
    }

    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Not valid JSON: {}", e))?;

    let filename_contains_project = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase().contains("project"))
        .unwrap_or(false);

    let name = if filename_contains_project {
        extract_first_row_title(&json)
            .or_else(|| extract_project_name(&json))
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unnamed")
                    .to_string()
            })
    } else {
        extract_project_name(&json).unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unnamed")
                .to_string()
        })
    };

    let cover_image = compress_cover_image_for_library(
        &file_path,
        extract_cover_image(&json).as_deref(),
        LIBRARY_COVER_IMAGE_MAX_BYTES,
    )?;
    let viewer_preference = detect_default_viewer_preference(&json);

    let project = Project {
        id: Uuid::new_v4().to_string(),
        name,
        description: String::new(),
        cover_image,
        source_url,
        file_path,
        viewer_preference,
        favorite: false,
        date_added: Utc::now().to_rfc3339(),
        tags: Vec::new(),
        file_missing: false,
    };

    insert_library_project(&project)?;

    let mut lib = state.lock().map_err(|e| e.to_string())?;
    lib.projects.push(project.clone());
    if let Err(error) = sync_index_for_project_if_present(&project) {
        eprintln!("Failed to update perk index after add: {}", error);
    }
    Ok(project)
}

fn replace_project_from_path(
    project_id: &str,
    file_path: String,
    state: &State<LibraryState>,
    source_url: Option<String>,
    desired_name: &str,
) -> Result<Project, String> {
    let path = Path::new(&file_path);
    if !path.exists() {
        return Err(format!("File does not exist: {}", file_path));
    }

    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Not valid JSON: {}", e))?;

    let filename_contains_project = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase().contains("project"))
        .unwrap_or(false);

    let derived_name = if filename_contains_project {
        extract_first_row_title(&json)
            .or_else(|| extract_project_name(&json))
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unnamed")
                    .to_string()
            })
    } else {
        extract_project_name(&json).unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unnamed")
                .to_string()
        })
    };

    let cover_image = compress_cover_image_for_library(
        &file_path,
        extract_cover_image(&json).as_deref(),
        LIBRARY_COVER_IMAGE_MAX_BYTES,
    )?;
    let detected_viewer_preference = detect_default_viewer_preference(&json);

    let mut lib = state.lock().map_err(|e| e.to_string())?;
    let index = lib
        .projects
        .iter()
        .position(|p| p.id == project_id)
        .ok_or_else(|| format!("Project not found: {}", project_id))?;

    let mut updated = lib.projects[index].clone();
    updated.name = if desired_name.trim().is_empty() {
        derived_name
    } else {
        desired_name.trim().to_string()
    };
    updated.cover_image = cover_image;
    updated.source_url = source_url;
    updated.file_path = file_path;
    updated.file_missing = false;
    if updated.viewer_preference.is_none() {
        updated.viewer_preference = detected_viewer_preference;
    }

    persist_project(&updated)?;
    lib.projects[index] = updated.clone();
    if let Err(error) = sync_index_for_project_if_present(&updated) {
        eprintln!("Failed to update perk index after overwrite: {}", error);
    }
    Ok(updated)
}

fn existing_project_file_path(project_id: &str, state: &State<LibraryState>) -> Option<PathBuf> {
    let lib = state.lock().ok()?;
    lib.projects
        .iter()
        .find(|project| project.id == project_id)
        .map(|project| PathBuf::from(&project.file_path))
}

fn removable_project_target_path(project_file_path: &Path) -> PathBuf {
    let is_folder_project = project_file_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("project.json"))
        .unwrap_or(false);

    if is_folder_project {
        return project_file_path
            .parent()
            .map(|value| value.to_path_buf())
            .unwrap_or_else(|| project_file_path.to_path_buf());
    }

    project_file_path.to_path_buf()
}

fn normalize_source_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut url = tauri::Url::parse(trimmed).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }

    url.set_fragment(None);

    let last_segment = url
        .path_segments()
        .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
        .map(|segment| segment.to_ascii_lowercase());

    if matches!(last_segment.as_deref(), Some(segment) if segment.ends_with(".json")) {
        let mut segments = url.path_segments_mut().ok()?;
        segments.pop_if_empty();
        segments.pop();
    }

    if url.path().is_empty() {
        url.set_path("/");
    }

    Some(url.to_string())
}

#[tauri::command]
pub fn remove_project(id: String, state: State<LibraryState>) -> Result<(), String> {
    delete_library_project(&id)?;

    let mut lib = state.lock().map_err(|e| e.to_string())?;
    lib.projects.retain(|p| p.id != id);
    if let Err(error) = remove_project_from_index_if_present(&id) {
        eprintln!("Failed to update perk index after removal: {}", error);
    }
    Ok(())
}

#[tauri::command]
pub fn remove_project_from_disk(id: String, state: State<LibraryState>) -> Result<(), String> {
    let target_path = {
        let lib = state.lock().map_err(|e| e.to_string())?;
        let project = lib
            .projects
            .iter()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| format!("Project not found: {}", id))?;
        removable_project_target_path(Path::new(&project.file_path))
    };

    if target_path.exists() {
        if target_path.is_dir() {
            std::fs::remove_dir_all(&target_path)
                .map_err(|error| format!("Failed to delete project folder: {}", error))?;
        } else {
            std::fs::remove_file(&target_path)
                .map_err(|error| format!("Failed to delete project file: {}", error))?;
        }
    }

    delete_library_project(&id)?;

    let mut lib = state.lock().map_err(|e| e.to_string())?;
    lib.projects.retain(|project| project.id != id);
    if let Err(error) = remove_project_from_index_if_present(&id) {
        eprintln!("Failed to update perk index after disk removal: {}", error);
    }
    Ok(())
}

#[tauri::command]
pub fn clear_library(state: State<LibraryState>) -> Result<(), String> {
    clear_library_storage()?;

    let mut lib = state.lock().map_err(|e| e.to_string())?;
    lib.projects.clear();
    if let Err(error) = clear_index_if_present() {
        eprintln!("Failed to clear perk index: {}", error);
    }
    Ok(())
}

#[tauri::command]
pub fn update_project(
    id: String,
    patch: ProjectPatch,
    state: State<LibraryState>,
) -> Result<Project, String> {
    let mut lib = state.lock().map_err(|e| e.to_string())?;
    let index = lib
        .projects
        .iter()
        .position(|p| p.id == id)
        .ok_or_else(|| format!("Project not found: {}", id))?;

    let mut updated = lib.projects[index].clone();

    if let Some(name) = patch.name {
        updated.name = name;
    }
    if let Some(description) = patch.description {
        updated.description = description;
    }
    if let Some(cover_image) = patch.cover_image {
        updated.cover_image = if cover_image.is_empty() {
            None
        } else {
            Some(cover_image)
        };
    }
    if let Some(vp) = patch.viewer_preference {
        updated.viewer_preference = if vp.is_empty() { None } else { Some(vp) };
    }
    if let Some(favorite) = patch.favorite {
        updated.favorite = favorite;
    }
    if let Some(tags) = patch.tags {
        updated.tags = tags;
    }
    if let Some(fp) = patch.file_path {
        updated.file_missing = !Path::new(&fp).exists();
        updated.file_path = fp;
    }

    persist_project(&updated)?;
    lib.projects[index] = updated.clone();
    if let Err(error) = sync_index_for_project_if_present(&updated) {
        eprintln!("Failed to update perk index after edit: {}", error);
    }
    Ok(updated)
}

#[tauri::command]
pub fn set_project_favorite(
    id: String,
    favorite: bool,
    state: State<LibraryState>,
) -> Result<Project, String> {
    let mut lib = state.lock().map_err(|e| e.to_string())?;
    let project = lib
        .projects
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("Project not found: {}", id))?;

    project.favorite = favorite;
    let updated = project.clone();
    drop(lib);

    persist_project_favorite(&updated.id, updated.favorite)?;

    Ok(updated)
}

#[tauri::command]
pub fn set_project_viewer_preference(
    id: String,
    viewer_preference: String,
    state: State<LibraryState>,
) -> Result<Project, String> {
    let mut lib = state.lock().map_err(|e| e.to_string())?;
    let index = lib
        .projects
        .iter()
        .position(|p| p.id == id)
        .ok_or_else(|| format!("Project not found: {}", id))?;

    let trimmed = viewer_preference.trim();
    let next_viewer_preference = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };

    persist_project_viewer_preference(&id, next_viewer_preference.as_deref())?;

    let mut updated = lib.projects[index].clone();
    updated.viewer_preference = next_viewer_preference;
    lib.projects[index] = updated.clone();

    Ok(updated)
}

#[tauri::command]
pub fn start_apply_oversize_project_action(
    app: tauri::AppHandle,
    id: String,
    strategy: String,
) -> Result<String, String> {
    let strategy = OversizeStrategy::parse(&strategy)?;
    if matches!(strategy, OversizeStrategy::Ask) {
        return Err("Strategy 'ask' is not valid for oversize actions".to_string());
    }

    let task_id = Uuid::new_v4().to_string();
    let app_handle = app.clone();
    let task_id_for_thread = task_id.clone();
    let project_id_for_thread = id.clone();

    thread::spawn(move || {
        let outcome = apply_oversize_project_action_internal(&project_id_for_thread, strategy, &app_handle.state::<LibraryState>());
        match outcome {
            Ok(_) => {
                emit_oversize_action_progress(
                    app_handle,
                    OversizeActionProgress {
                        task_id: task_id_for_thread,
                        project_id: project_id_for_thread,
                        phase: "done".to_string(),
                        message: "Oversize action applied".to_string(),
                        done: true,
                        success: true,
                        error: None,
                    },
                );
            }
            Err(error) => {
                emit_oversize_action_progress(
                    app_handle,
                    OversizeActionProgress {
                        task_id: task_id_for_thread,
                        project_id: project_id_for_thread,
                        phase: "error".to_string(),
                        message: "Oversize action failed".to_string(),
                        done: true,
                        success: false,
                        error: Some(error),
                    },
                );
            }
        }
    });

    Ok(task_id)
}

#[tauri::command]
pub fn apply_oversize_project_action(
    id: String,
    strategy: String,
    state: State<LibraryState>,
) -> Result<Project, String> {
    let strategy = OversizeStrategy::parse(&strategy)?;

    apply_oversize_project_action_internal(&id, strategy, &state)
}

fn apply_oversize_project_action_internal(
    id: &str,
    strategy: OversizeStrategy,
    state: &State<LibraryState>,
) -> Result<Project, String> {
    let (project_path, project_id) = {
        let lib = state.lock().map_err(|e| e.to_string())?;
        let project = lib
            .projects
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("Project not found: {}", id))?;
        (project.file_path.clone(), project.id.clone())
    };

    if matches!(strategy, OversizeStrategy::DoNothing) {
        let lib = state.lock().map_err(|e| e.to_string())?;
        let project = lib
            .projects
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("Project not found: {}", id))?;
        return Ok(project.clone());
    }

    let bytes = std::fs::read(&project_path)
        .map_err(|e| format!("Failed to read project file: {}", e))?;

    let new_path = match strategy {
        OversizeStrategy::KeepSeparate => {
            let extracted_path = extract_project_images_to_folder(&project_path, &bytes)?;
            Some(extracted_path)
        }
        OversizeStrategy::Compress => {
            let compressed = compress_project_file_images(&bytes)?;
            std::fs::write(&project_path, compressed)
                .map_err(|e| format!("Failed to write compressed project: {}", e))?;
            None
        }
        OversizeStrategy::Ask => {
            return Err("Strategy 'ask' is not allowed in apply_oversize_project_action".to_string())
        }
        OversizeStrategy::DoNothing => None,
    };

    let mut lib = state.lock().map_err(|e| e.to_string())?;
    let index = lib
        .projects
        .iter()
        .position(|p| p.id == project_id)
        .ok_or_else(|| format!("Project not found: {}", project_id))?;

    let mut updated = lib.projects[index].clone();
    if let Some(path) = new_path {
        updated.file_path = path.to_string_lossy().to_string();
    }

    persist_project(&updated)?;
    lib.projects[index] = updated.clone();
    Ok(updated)
}
#[tauri::command]
pub fn get_project_json(id: String, state: State<LibraryState>) -> Result<String, String> {
    let lib = state.lock().map_err(|e| e.to_string())?;
    let project = lib
        .projects
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("Project not found: {}", id))?;
    std::fs::read_to_string(&project.file_path).map_err(|e| e.to_string())
}

// ─── File Discovery ───────────────────────────────────────────────────────────

#[tauri::command]
pub fn scan_folder(folder: String) -> Vec<String> {
    use walkdir::WalkDir;

    WalkDir::new(&folder)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file() && is_bulk_import_project_file(e.path())
        })
        .filter_map(|e| e.path().to_str().map(|s| s.to_string()))
        .collect()
}

#[tauri::command]
pub fn start_scan_folder(app: tauri::AppHandle, folder: String) -> Result<String, String> {
    let folder_path = PathBuf::from(&folder);
    if !folder_path.exists() {
        return Err(format!("Folder does not exist: {}", folder));
    }
    if !folder_path.is_dir() {
        return Err(format!("Path is not a folder: {}", folder));
    }

    let task_id = Uuid::new_v4().to_string();
    let app_handle = app.clone();
    let task_id_for_thread = task_id.clone();

    thread::spawn(move || {
        match scan_folder_with_progress(&app_handle, &task_id_for_thread, &folder_path) {
            Ok((paths, scanned)) => emit_bulk_scan_progress(
                app_handle,
                BulkScanProgress {
                    task_id: task_id_for_thread,
                    scanned,
                    found: paths.len(),
                    message: format!("Scan complete. Found {} project files.", paths.len()),
                    done: true,
                    success: true,
                    error: None,
                    paths,
                },
            ),
            Err(error) => emit_bulk_scan_progress(
                app_handle,
                BulkScanProgress {
                    task_id: task_id_for_thread,
                    scanned: 0,
                    found: 0,
                    message: "Scan failed.".to_string(),
                    done: true,
                    success: false,
                    error: Some(error),
                    paths: Vec::new(),
                },
            ),
        }
    });

    Ok(task_id)
}

fn scan_folder_with_progress(
    app: &tauri::AppHandle,
    task_id: &str,
    folder: &Path,
) -> Result<(Vec<String>, usize), String> {
    use walkdir::WalkDir;

    let mut discovered = Vec::new();
    let mut scanned = 0usize;

    emit_bulk_scan_progress(
        app.clone(),
        BulkScanProgress {
            task_id: task_id.to_string(),
            scanned,
            found: discovered.len(),
            message: "Scanning folder for project JSON files…".to_string(),
            done: false,
            success: false,
            error: None,
            paths: Vec::new(),
        },
    );

    for entry in WalkDir::new(folder).into_iter() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };

        if !entry.file_type().is_file() {
            continue;
        }

        scanned += 1;
        let path = entry.path();

        if is_bulk_import_project_file(path) {
            if let Some(path_string) = path.to_str().map(|value| value.to_string()) {
                discovered.push(path_string);
            }

            emit_bulk_scan_progress(
                app.clone(),
                BulkScanProgress {
                    task_id: task_id.to_string(),
                    scanned,
                    found: discovered.len(),
                    message: format!("Scanning… {} found", discovered.len()),
                    done: false,
                    success: false,
                    error: None,
                    paths: Vec::new(),
                },
            );
            continue;
        }

        if scanned % 250 == 0 {
            emit_bulk_scan_progress(
                app.clone(),
                BulkScanProgress {
                    task_id: task_id.to_string(),
                    scanned,
                    found: discovered.len(),
                    message: format!("Scanning… {} found", discovered.len()),
                    done: false,
                    success: false,
                    error: None,
                    paths: Vec::new(),
                },
            );
        }
    }

    Ok((discovered, scanned))
}

fn is_bulk_import_project_file(path: &Path) -> bool {
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
    {
        return false;
    }

    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };

    json.get("rows")
        .and_then(|value| value.as_array())
        .is_some()
}

// ─── Viewers ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_viewers(app: tauri::AppHandle) -> Vec<Viewer> {
    let base = viewers_base_dir(Some(&app));
    let mut viewers = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().join("index.html").exists() {
                    viewers.push(Viewer {
                        id: slugify(&name),
                        name,
                    });
                }
            }
        }
    }
    viewers
}

#[tauri::command]
pub fn open_viewer_window(
    app: tauri::AppHandle,
    project_id: String,
    viewer_id: String,
    project_name: String,
    cheats_enabled: bool,
    sessions: State<SessionStore>,
) -> Result<(), String> {
    let label = format!("viewer-{}", &Uuid::new_v4().to_string()[..8]);

    {
        let mut store = sessions.lock().map_err(|e| e.to_string())?;
        store.insert(
            label.clone(),
            ViewerSession {
                project_id,
                viewer_id,
                cheats_enabled,
            },
        );
    }

    let url = tauri::Url::parse("cyoaview://localhost/index.html").map_err(|e| e.to_string())?;

    let app_for_thread = app.clone();
    let label_for_thread = label.clone();
    let project_name_for_thread = project_name.clone();
    std::thread::spawn(move || {
        if let Ok(window) = tauri::WebviewWindowBuilder::new(
            &app_for_thread,
            &label_for_thread,
            tauri::WebviewUrl::CustomProtocol(url),
        )
        .title(&project_name_for_thread)
        .inner_size(1920.0, 1080.0)
        .maximized(true)
        .zoom_hotkeys_enabled(true)
        .build()
        {
            let _ = window;
        }
    });

    Ok(())
}

fn ensure_project_within_size_limit(
    project_id: &str,
    max_project_size_bytes: u64,
    state: &State<LibraryState>,
) -> Result<(), String> {
    let lib = state.lock().map_err(|e| e.to_string())?;
    let project = lib
        .projects
        .iter()
        .find(|candidate| candidate.id == project_id)
        .ok_or_else(|| format!("Project not found after import: {}", project_id))?;

    let metadata = std::fs::metadata(&project.file_path)
        .map_err(|e| format!("Failed to read project metadata: {}", e))?;
    let size = metadata.len();

    if size > max_project_size_bytes {
        return Err(format!("OVERSIZE|{}|{}|{}", project.id, size, max_project_size_bytes));
    }

    Ok(())
}
fn download_file_name(url: &tauri::Url) -> String {
    let mut base = url
        .path_segments()
        .and_then(|segments| {
            let segments: Vec<_> = segments.collect();
            segments
                .iter()
                .rev()
                .find(|segment| !segment.is_empty() && !segment.eq_ignore_ascii_case("project.json"))
                .map(|segment| (*segment).to_string())
        })
        .or_else(|| url.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "downloaded-cyoa".to_string());

    if slugify(&base).is_empty() {
        base = "downloaded-cyoa".to_string();
    }

    format!("{}-project.json", slugify(&base))
}

fn unique_path(path: std::path::PathBuf) -> std::path::PathBuf {
    if !path.exists() {
        return path;
    }

    let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("json");

    for i in 2..10000 {
        let candidate = parent.join(format!("{}-{}.{}", stem, i, ext));
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(format!("{}-{}.{}", stem, Uuid::new_v4(), ext))
}

fn unique_dir_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }

    let parent = path.parent().map(|value| value.to_path_buf()).unwrap_or_default();
    let base = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("project")
        .to_string();

    for index in 2..10000 {
        let candidate = parent.join(format!("{}-{}", base, index));
        if !candidate.exists() {
            return candidate;
        }
    }

    parent.join(format!("{}-{}", base, Uuid::new_v4()))
}

fn inline_downloaded_project_images(
    bytes: &[u8],
    base_url: &tauri::Url,
    app: &tauri::AppHandle,
    task_id: &str,
) -> Result<Vec<u8>, String> {
    let mut json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("Downloaded file is not valid JSON: {}", e))?;
    let image_refs = collect_image_refs(&json, base_url);
    let image_total = image_refs.len();

    emit_download_progress(
        app.clone(),
        DownloadProjectProgress {
            task_id: task_id.to_string(),
            phase: "scanning-images".to_string(),
            current: 0,
            total: 1,
            image_current: 0,
            image_total,
            message: if image_total == 0 {
                "No linked images found".to_string()
            } else {
                format!("Found {} linked images", image_total)
            },
            done: false,
            success: false,
            error: None,
        },
    );

    let cache = download_images_parallel(app, task_id, image_refs)?;
    replace_image_refs(&mut json, base_url, &cache);
    serde_json::to_vec(&json).map_err(|e| format!("Failed to serialize downloaded project: {}", e))
}

fn inline_downloaded_project_images_silent(
    bytes: &[u8],
    base_url: &tauri::Url,
) -> Result<Vec<u8>, String> {
    let mut json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("Downloaded file is not valid JSON: {}", e))?;
    let image_refs = collect_image_refs(&json, base_url);

    if image_refs.is_empty() {
        return serde_json::to_vec(&json)
            .map_err(|e| format!("Failed to serialize downloaded project: {}", e));
    }

    let mut cache = HashMap::new();
    for url in image_refs {
        if let Some(data) = download_image_as_data_uri(&url) {
            cache.insert(url.to_string(), data);
        }
    }

    replace_image_refs(&mut json, base_url, &cache);
    serde_json::to_vec(&json).map_err(|e| format!("Failed to serialize downloaded project: {}", e))
}

fn extract_project_images_to_folder(project_file_path: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let mut json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("Failed to parse project for image extraction: {}", e))?;

    let source_path = Path::new(project_file_path);
    let source_dir = source_path
        .parent()
        .ok_or_else(|| "Project path has no parent directory".to_string())?;
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("project")
        .to_string();

    let project_folder = unique_dir_path(source_dir.join(format!("{}-separate", stem)));
    let images_folder = project_folder.join("images");
    std::fs::create_dir_all(&images_folder)
        .map_err(|e| format!("Failed to create images folder: {}", e))?;

    let mut image_index = 0usize;
    rewrite_data_images_to_files(&mut json, &images_folder, &mut image_index)?;

    let serialized = serde_json::to_vec(&json)
        .map_err(|e| format!("Failed to serialize project JSON: {}", e))?;
    let project_json_path = project_folder.join("project.json");
    std::fs::write(&project_json_path, serialized)
        .map_err(|e| format!("Failed to write separated project JSON: {}", e))?;

    Ok(project_json_path)
}

fn compress_project_file_images(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("Failed to parse project for compression: {}", e))?;

    compress_data_uri_images_in_json(&mut json, 150 * 1024)?;

    serde_json::to_vec(&json)
        .map_err(|e| format!("Failed to serialize compressed project: {}", e))
}

fn rewrite_data_images_to_files(
    value: &mut serde_json::Value,
    images_folder: &Path,
    image_index: &mut usize,
) -> Result<(), String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if let serde_json::Value::String(text) = child {
                    if is_image_field(key) && text.trim_start().starts_with("data:") {
                        if let Some((mime, bytes)) = parse_data_uri_image(text) {
                            *image_index += 1;
                            let ext = extension_for_mime(&mime);
                            let filename = format!("img-{:05}.{}", image_index, ext);
                            let image_path = images_folder.join(&filename);
                            std::fs::write(&image_path, bytes)
                                .map_err(|e| format!("Failed writing extracted image: {}", e))?;
                            *text = format!("images/{}", filename);
                        }
                    }
                } else {
                    rewrite_data_images_to_files(child, images_folder, image_index)?;
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                rewrite_data_images_to_files(item, images_folder, image_index)?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn compress_data_uri_images_in_json(value: &mut serde_json::Value, max_bytes: usize) -> Result<(), String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if let serde_json::Value::String(text) = child {
                    if is_image_field(key) && text.trim_start().starts_with("data:") {
                        if let Some((mime, bytes)) = parse_data_uri_image(text) {
                            if let Some(compressed) = compress_image_to_jpeg_limit(&bytes, max_bytes) {
                                let encoded = base64::engine::general_purpose::STANDARD.encode(compressed);
                                *text = format!("data:image/jpeg;base64,{}", encoded);
                            } else {
                                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                                *text = format!("data:{};base64,{}", mime, encoded);
                            }
                        }
                    }
                } else {
                    compress_data_uri_images_in_json(child, max_bytes)?;
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                compress_data_uri_images_in_json(item, max_bytes)?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn parse_data_uri_image(data_uri: &str) -> Option<(String, Vec<u8>)> {
    let trimmed = data_uri.trim();
    if !trimmed.starts_with("data:") {
        return None;
    }

    let mut parts = trimmed[5..].splitn(2, ',');
    let header = parts.next()?;
    let payload = parts.next()?;
    if !header.to_ascii_lowercase().contains(";base64") {
        return None;
    }

    let mime = header
        .split(';')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("application/octet-stream")
        .to_string();

    let bytes = base64::engine::general_purpose::STANDARD.decode(payload).ok()?;
    Some((mime, bytes))
}

fn extension_for_mime(mime: &str) -> &'static str {
    let lower = mime.to_ascii_lowercase();
    if lower.contains("png") {
        "png"
    } else if lower.contains("avif") {
        "avif"
    } else if lower.contains("gif") {
        "gif"
    } else if lower.contains("webp") {
        "webp"
    } else if lower.contains("bmp") {
        "bmp"
    } else if lower.contains("svg") {
        "svg"
    } else if lower.contains("tiff") || lower.contains("tif") {
        "tiff"
    } else {
        "jpg"
    }
}

fn compress_image_to_jpeg_limit(source: &[u8], max_bytes: usize) -> Option<Vec<u8>> {
    let image = image::load_from_memory(source).ok()?;
    let flattened = flatten_alpha(image);

    let mut quality = 85u8;
    while quality >= 35 {
        let mut output = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut output, quality);
        if encoder.encode_image(&flattened).is_ok() && output.len() <= max_bytes {
            return Some(output);
        }

        if quality <= 35 {
            break;
        }
        quality = quality.saturating_sub(10);
    }

    let mut fallback = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut fallback, 35);
    if encoder.encode_image(&flattened).is_ok() {
        return Some(fallback);
    }

    None
}

fn flatten_alpha(image: DynamicImage) -> DynamicImage {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut rgb = image::RgbImage::new(width, height);

    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = pixel[3] as u16;
        let inv = 255u16.saturating_sub(alpha);
        let r = ((pixel[0] as u16 * alpha + 255u16 * inv) / 255u16) as u8;
        let g = ((pixel[1] as u16 * alpha + 255u16 * inv) / 255u16) as u8;
        let b = ((pixel[2] as u16 * alpha + 255u16 * inv) / 255u16) as u8;
        rgb.put_pixel(x, y, image::Rgb([r, g, b]));
    }

    DynamicImage::ImageRgb8(rgb)
}

fn collect_image_refs(
    value: &serde_json::Value,
    base_url: &tauri::Url,
 ) -> Vec<tauri::Url> {
    let mut refs = HashSet::new();
    collect_image_refs_inner(value, base_url, &mut refs);
    refs.into_iter().collect()
}

fn collect_image_refs_inner(
    value: &serde_json::Value,
    base_url: &tauri::Url,
    refs: &mut HashSet<tauri::Url>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if let serde_json::Value::String(text) = child {
                    if is_image_field(key) {
                        if let Some(url) = resolve_remote_asset_url(base_url, text.trim()) {
                            refs.insert(url);
                        }
                    }
                } else {
                    collect_image_refs_inner(child, base_url, refs);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_image_refs_inner(item, base_url, refs);
            }
        }
        _ => {}
    }
}

fn replace_image_refs(
    value: &mut serde_json::Value,
    base_url: &tauri::Url,
    cache: &HashMap<String, String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if let serde_json::Value::String(text) = child {
                    if is_image_field(key) {
                        let trimmed = text.trim();
                        if let Some(resolved) = resolve_remote_asset_url(base_url, trimmed) {
                            if let Some(inlined) = cache.get(resolved.as_str()) {
                                *text = inlined.clone();
                            }
                        }
                    }
                } else {
                    replace_image_refs(child, base_url, cache);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                replace_image_refs(item, base_url, cache);
            }
        }
        _ => {}
    }
}

fn is_image_field(key: &str) -> bool {
    key.to_ascii_lowercase().contains("image")
}

fn resolve_remote_asset_url(base_url: &tauri::Url, raw: &str) -> Option<tauri::Url> {
    if raw.is_empty() || raw.starts_with("data:") {
        return None;
    }

    if let Ok(url) = tauri::Url::parse(raw) {
        if matches!(url.scheme(), "http" | "https") {
            return Some(url);
        }
    }

    if raw.starts_with("//") {
        return tauri::Url::parse(&format!("{}:{}", base_url.scheme(), raw)).ok();
    }

    base_url.join(raw).ok().filter(|url| matches!(url.scheme(), "http" | "https"))
}

fn resolve_cover_image_source(
    project_file_path: &str,
    cover_image: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(raw_cover_image) = cover_image.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if is_remote_or_embedded_image(raw_cover_image) {
        return Ok(Some(raw_cover_image.to_string()));
    }

    let image_path = resolve_cover_image_path(project_file_path, raw_cover_image);
    if !image_path.exists() {
        return Ok(None);
    }

    let mime = mime_guess::from_path(&image_path)
        .first_raw()
        .unwrap_or("application/octet-stream");
    let bytes = std::fs::read(&image_path).map_err(|e| e.to_string())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(Some(format!("data:{};base64,{}", mime, encoded)))
}

fn is_remote_or_embedded_image(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("data:")
        || lower.starts_with("blob:")
}

fn resolve_cover_image_path(project_file_path: &str, cover_image: &str) -> PathBuf {
    let cover_path = PathBuf::from(cover_image);
    if cover_path.is_absolute() {
        return cover_path;
    }

    Path::new(project_file_path)
        .parent()
        .map(|parent| parent.join(&cover_path))
        .unwrap_or(cover_path)
}

fn download_image_as_data_uri(url: &tauri::Url) -> Option<String> {
    let response = reqwest::blocking::get(url.clone()).ok()?.error_for_status().ok()?;
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .or_else(|| {
            mime_guess::from_path(url.path())
                .first_raw()
                .map(|value| value.to_string())
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let bytes = response.bytes().ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{};base64,{}", mime, encoded))
}

fn download_images_parallel(
    app: &tauri::AppHandle,
    task_id: &str,
    image_urls: Vec<tauri::Url>,
) -> Result<HashMap<String, String>, String> {
    if image_urls.is_empty() {
        return Ok(HashMap::new());
    }

    let total = image_urls.len();
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(6)
        .min(total.max(1));
    let chunk_size = total.div_ceil(workers);
    let (tx, rx) = mpsc::channel();

    for chunk in image_urls.chunks(chunk_size) {
        let tx = tx.clone();
        let urls = chunk.to_vec();
        thread::spawn(move || {
            for url in urls {
                let result = download_image_as_data_uri(&url);
                let _ = tx.send((url, result));
            }
        });
    }
    drop(tx);

    let mut completed = 0;
    let mut cache = HashMap::new();
    for (url, data) in rx {
        completed += 1;
        if let Some(data) = data {
            cache.insert(url.to_string(), data);
        }

        emit_download_progress(
            app.clone(),
            DownloadProjectProgress {
                task_id: task_id.to_string(),
                phase: "downloading-images".to_string(),
                current: completed,
                total,
                image_current: completed,
                image_total: total,
                message: format!("Downloaded {}/{} images", completed, total),
                done: false,
                success: false,
                error: None,
            },
        );
    }

    Ok(cache)
}

fn emit_download_progress(app: tauri::AppHandle, payload: DownloadProjectProgress) {
    let _ = app.emit("download-project-progress", payload);
}

fn emit_catalog_import_progress(app: tauri::AppHandle, payload: CatalogImportProgress) {
    let _ = app.emit("download-catalog-progress", payload);
}

fn emit_oversize_action_progress(app: tauri::AppHandle, payload: OversizeActionProgress) {
    let _ = app.emit("oversize-action-progress", payload);
}

fn emit_bulk_scan_progress(app: tauri::AppHandle, payload: BulkScanProgress) {
    let _ = app.emit("bulk-scan-progress", payload);
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn extract_project_name(json: &serde_json::Value) -> Option<String> {
    json.get("title")
        .or_else(|| json.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn format_bytes_megabytes(bytes: u64) -> String {
    let megabytes = bytes as f64 / (1024.0 * 1024.0);
    format!("{:.1} MB", megabytes)
}

fn extract_cover_image(json: &serde_json::Value) -> Option<String> {
    // Check top-level image field
    if let Some(img) = json.get("image").and_then(|v| v.as_str()) {
        if !img.is_empty() {
            return Some(img.to_string());
        }
    }
    // Scan first few rows for an image
    if let Some(rows) = json.get("rows").and_then(|r| r.as_array()) {
        for row in rows.iter().take(5) {
            if let Some(img) = row.get("image").and_then(|v| v.as_str()) {
                if !img.is_empty() {
                    return Some(img.to_string());
                }
            }
        }
    }
    None
}

fn compress_cover_image_for_library(
    project_file_path: &str,
    cover_image: Option<&str>,
    max_bytes: usize,
) -> Result<Option<String>, String> {
    let Some(raw_cover_image) = cover_image.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if raw_cover_image.starts_with("http://") || raw_cover_image.starts_with("https://") || raw_cover_image.starts_with("blob:") {
        return Ok(Some(raw_cover_image.to_string()));
    }

    if raw_cover_image.starts_with("data:") {
        if let Some((_, bytes)) = parse_data_uri_image(raw_cover_image) {
            if bytes.len() > max_bytes {
                if let Some(compressed) = compress_image_to_jpeg_limit(&bytes, max_bytes) {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(compressed);
                    return Ok(Some(format!("data:image/jpeg;base64,{}", encoded)));
                }
            }
        }

        return Ok(Some(raw_cover_image.to_string()));
    }

    let image_path = resolve_cover_image_path(project_file_path, raw_cover_image);
    if !image_path.exists() {
        return Ok(Some(raw_cover_image.to_string()));
    }

    let bytes = std::fs::read(&image_path).map_err(|e| e.to_string())?;
    if bytes.len() <= max_bytes {
        return Ok(Some(raw_cover_image.to_string()));
    }

    if let Some(compressed) = compress_image_to_jpeg_limit(&bytes, max_bytes) {
        let encoded = base64::engine::general_purpose::STANDARD.encode(compressed);
        return Ok(Some(format!("data:image/jpeg;base64,{}", encoded)));
    }

    Ok(Some(raw_cover_image.to_string()))
}

fn extract_first_row_title(json: &serde_json::Value) -> Option<String> {
    json.get("rows")
        .and_then(|rows| rows.as_array())
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("title"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Returns the directory containing viewer sub-folders.
/// Dev:  `<workspace>/public/viewers`
/// Prod: resolves bundled resources via Tauri, then falls back to legacy layouts.
pub fn viewers_base_dir(_app_handle: Option<&tauri::AppHandle>) -> std::path::PathBuf {
    #[cfg(debug_assertions)]
    {
        // CARGO_MANIFEST_DIR is src-tauri/; one level up is the workspace root
        let manifest = env!("CARGO_MANIFEST_DIR");
        std::path::Path::new(manifest)
            .parent()
            .unwrap()
            .join("public")
            .join("viewers")
    }
    #[cfg(not(debug_assertions))]
    {
        if let Some(app) = _app_handle {
            if let Ok(resource_dir) = app.path().resource_dir() {
                let resource_viewers = resource_dir.join("viewers");
                if resource_viewers.exists() {
                    return resource_viewers;
                }
            }
        }

        let exe = std::env::current_exe().expect("cannot resolve exe path");
        let exe_dir = exe.parent().expect("exe has no parent");

        // Probe common packaging layouts across platforms.
        let mut candidates = vec![
            exe_dir.join("viewers"),
            exe_dir.join("_up_").join("public").join("viewers"),
            exe_dir.join("resources").join("viewers"),
            exe_dir.join("resources").join("public").join("viewers"),
            exe_dir
                .join("resources")
                .join("_up_")
                .join("public")
                .join("viewers"),
        ];

        if let Some(contents_dir) = exe_dir.parent() {
            candidates.push(contents_dir.join("Resources").join("viewers"));
            candidates.push(contents_dir.join("Resources").join("resources").join("viewers"));
            candidates.push(contents_dir.join("Resources").join("public").join("viewers"));
            candidates.push(
                contents_dir
                    .join("Resources")
                    .join("_up_")
                    .join("public")
                    .join("viewers"),
            );
        }

        for candidate in &candidates {
            if candidate.exists() {
                return candidate.clone();
            }
        }

        candidates
            .into_iter()
            .next()
            .unwrap_or_else(|| exe_dir.join("viewers"))
    }
}

/// Convert an arbitrary folder name to a URL-safe slug.
pub fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Reload the library from disk (used after external edits).
#[allow(dead_code)]
pub fn sync_library(state: &State<LibraryState>) {
    if let Ok(fresh) = state.lock() {
        drop(fresh); // release before reload
    }
    let Ok(fresh) = reload_library() else {
        return;
    };
    if let Ok(mut lib) = state.lock() {
        *lib = fresh;
    }
}
