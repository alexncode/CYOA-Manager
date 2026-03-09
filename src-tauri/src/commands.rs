use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;

use base64::Engine;
use tauri::{Emitter, Manager, State};
use uuid::Uuid;
use chrono::Utc;

use crate::library::{cyoas_dir, load_library as reload_from_disk, save_library};
use crate::models::{Library, Project, ProjectPatch, SessionStore, Viewer, ViewerSession};

pub type LibraryState = Mutex<Library>;

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
pub fn add_project(file_path: String, state: State<LibraryState>) -> Result<Project, String> {
    add_project_from_path(file_path, &state)
}

#[tauri::command]
pub fn resolve_cover_image_src(
    file_path: String,
    cover_image: Option<String>,
) -> Result<Option<String>, String> {
    resolve_cover_image_source(&file_path, cover_image.as_deref())
}

#[tauri::command]
pub fn start_download_project(app: tauri::AppHandle, url: String) -> Result<String, String> {
    let task_id = Uuid::new_v4().to_string();
    let app_handle = app.clone();
    let task_id_for_thread = task_id.clone();

    thread::spawn(move || {
        if let Err(error) = run_download_project(&app_handle, &task_id_for_thread, url) {
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
pub fn download_catalog_entry(
    website_url: String,
    zip_url: String,
    project_name: String,
    state: State<LibraryState>,
) -> Result<Project, String> {
    let desired_name = project_name.trim();

    let mut project = match import_project_from_catalog_website(
        None,
        website_url.trim(),
        desired_name,
        &state,
    ) {
        Ok(project) => project,
        Err(_) => {
            let extracted_project = download_catalog_project_zip(zip_url.trim(), desired_name, None)?;
            add_project_from_path(extracted_project.to_string_lossy().to_string(), &state)?
        }
    };

    apply_catalog_project_name_override(&mut project, desired_name, &state)?;
    Ok(project)
}

#[tauri::command]
pub fn start_download_catalog_entry(
    app: tauri::AppHandle,
    task_id: String,
    website_url: String,
    zip_url: String,
    project_name: String,
) -> Result<String, String> {
    let task_id = task_id.trim().to_string();
    if task_id.is_empty() {
        return Err("Missing task id".to_string());
    }
    let app_handle = app.clone();
    let task_id_for_thread = task_id.clone();

    thread::spawn(move || {
        if let Err(error) = run_download_catalog_entry(
            &app_handle,
            &task_id_for_thread,
            website_url,
            zip_url,
            project_name,
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

fn run_download_project(app: &tauri::AppHandle, task_id: &str, url: String) -> Result<(), String> {
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

    let (bytes, base_url) = download_project_data(parsed)?;

    let processed = inline_downloaded_project_images(bytes.as_ref(), &base_url, app, task_id)?;

    let dir = cyoas_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create cyoas folder: {}", e))?;

    let destination = unique_path(dir.join(download_file_name(&base_url)));
    std::fs::write(&destination, processed).map_err(|e| format!("Failed to save file: {}", e))?;

    let library = app.state::<LibraryState>();
    let project = add_project_from_path(destination.to_string_lossy().to_string(), &library)?;

    emit_download_progress(
        app.clone(),
        DownloadProjectProgress {
            task_id: task_id.to_string(),
            phase: "done".to_string(),
            current: 1,
            total: 1,
            image_current: 1,
            image_total: 1,
            message: format!("Imported {}", project.name),
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

    let mut project = match import_project_from_catalog_website(
        Some((app, task_id)),
        website_url.trim(),
        desired_name,
        &library,
    ) {
        Ok(project) => project,
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

            let extracted_project = download_catalog_project_zip(
                zip_url.trim(),
                desired_name,
                Some((app, task_id)),
            )?;

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

            add_project_from_path(extracted_project.to_string_lossy().to_string(), &library)?
        }
    };

    apply_catalog_project_name_override(&mut project, desired_name, &library)?;

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

fn import_project_from_catalog_website(
    progress: Option<(&tauri::AppHandle, &str)>,
    website_url: &str,
    project_name: &str,
    state: &State<LibraryState>,
) -> Result<Project, String> {
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

    let (bytes, base_url) = download_project_data(parsed)?;

    if let Some((app, task_id)) = progress {
        emit_catalog_import_progress(
            app.clone(),
            CatalogImportProgress {
                task_id: task_id.to_string(),
                phase: "processing-project".to_string(),
                current: 55,
                total: 100,
                message: format!("Processing linked images for {}", project_name),
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

    let destination = unique_path(dir.join(download_file_name(&base_url)));
    std::fs::write(&destination, processed).map_err(|e| format!("Failed to save file: {}", e))?;

    add_project_from_path(destination.to_string_lossy().to_string(), state)
}

fn download_project_data(url: tauri::Url) -> Result<(Vec<u8>, tauri::Url), String> {
    if is_cyoa_cafe_host(url.host_str()) {
        return download_cyoa_cafe_project_data(url);
    }

    if is_itch_io_host(url.host_str()) {
        return download_itch_io_project_data(url);
    }

    if is_direct_project_json_url(&url) {
        return download_project_json_bytes(url);
    }

    let default_project_url = build_default_project_json_url(&url);
    if let Ok(project) = download_project_json_bytes(default_project_url) {
        return Ok(project);
    }

    let (html, final_page_url) = fetch_html_page(url, "Failed to open website")?;

    if let Some(project_url) = find_project_json_url_in_html(&html, &final_page_url) {
        if let Ok(project) = download_project_json_bytes(project_url) {
            return Ok(project);
        }
    }

    if let Some((json, base_url)) = find_embedded_project_data(&html, &final_page_url)? {
        let bytes = serde_json::to_vec(&json)
            .map_err(|e| format!("Failed to serialize embedded project data: {}", e))?;
        return Ok((bytes, base_url));
    }

    Err("Could not find project.json or embedded project data on the provided page".to_string())
}

fn download_itch_io_project_data(url: tauri::Url) -> Result<(Vec<u8>, tauri::Url), String> {
    let (html, final_url) = fetch_html_page(url, "Failed to open itch.io page")?;

    if let Some(iframe_url) = find_itch_io_iframe_url_in_html(&html, &final_url) {
        if iframe_url != final_url {
            return download_project_data(iframe_url);
        }
    }

    if let Some(project_url) = find_project_json_url_in_html(&html, &final_url) {
        if let Ok(project) = download_project_json_bytes(project_url) {
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

fn download_cyoa_cafe_project_data(url: tauri::Url) -> Result<(Vec<u8>, tauri::Url), String> {
    if let Some(game_id) = extract_cyoa_cafe_game_id(&url) {
        if let Some(source_url) = fetch_cyoa_cafe_game_source_url(&game_id)? {
            let parsed_source = tauri::Url::parse(source_url.trim())
                .map_err(|e| format!("Invalid source URL from cyoa.cafe: {}", e))?;
            return download_project_data(parsed_source);
        }
    }

    let (html, final_url) = fetch_html_page(url, "Failed to open cyoa.cafe page")?;
    if let Some(project_url) = find_project_json_url_in_html(&html, &final_url) {
        if let Ok(project) = download_project_json_bytes(project_url) {
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

fn download_project_json_bytes(url: tauri::Url) -> Result<(Vec<u8>, tauri::Url), String> {
    let response = reqwest::blocking::get(url.clone())
        .map_err(|e| format!("Download failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {}", e))?;

    let final_url = tauri::Url::parse(response.url().as_str())
        .map_err(|e| format!("Failed to parse downloaded URL: {}", e))?;
    let bytes = response
        .bytes()
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    serde_json::from_slice::<serde_json::Value>(bytes.as_ref())
        .map_err(|e| format!("Downloaded file was not valid JSON: {}", e))?;

    Ok((bytes.to_vec(), final_url))
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

fn find_embedded_project_data(
    html: &str,
    page_url: &tauri::Url,
) -> Result<Option<(serde_json::Value, tauri::Url)>, String> {
    for script in extract_inline_script_blocks(html) {
        if let Some(project) = extract_embedded_project_json(script) {
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
    let stored = lib
        .projects
        .iter_mut()
        .find(|candidate| candidate.id == project.id)
        .ok_or_else(|| format!("Project not found after import: {}", project.id))?;
    stored.name = desired_name.to_string();
    project.name = stored.name.clone();
    save_library(&lib)?;
    Ok(())
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
    let bytes = response
        .bytes()
        .map_err(|e| format!("Failed to read ZIP archive: {}", e))?;

    if let Some((app, task_id)) = progress {
        emit_catalog_import_progress(
            app.clone(),
            CatalogImportProgress {
                task_id: task_id.to_string(),
                phase: "extracting-archive".to_string(),
                current: 25,
                total: 100,
                message: format!("Extracting {}", project_name),
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

fn add_project_from_path(file_path: String, state: &State<LibraryState>) -> Result<Project, String> {
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

    let cover_image = extract_cover_image(&json);
    let viewer_preference = detect_default_viewer_preference(&json);

    let project = Project {
        id: Uuid::new_v4().to_string(),
        name,
        description: String::new(),
        cover_image,
        file_path,
        viewer_preference,
        date_added: Utc::now().to_rfc3339(),
        tags: Vec::new(),
        file_missing: false,
    };

    let mut lib = state.lock().map_err(|e| e.to_string())?;
    lib.projects.push(project.clone());
    save_library(&lib)?;
    Ok(project)
}

#[tauri::command]
pub fn remove_project(id: String, state: State<LibraryState>) -> Result<(), String> {
    let mut lib = state.lock().map_err(|e| e.to_string())?;
    lib.projects.retain(|p| p.id != id);
    save_library(&lib)
}

#[tauri::command]
pub fn clear_library(state: State<LibraryState>) -> Result<(), String> {
    let mut lib = state.lock().map_err(|e| e.to_string())?;
    lib.projects.clear();
    save_library(&lib)
}

#[tauri::command]
pub fn update_project(
    id: String,
    patch: ProjectPatch,
    state: State<LibraryState>,
) -> Result<Project, String> {
    let mut lib = state.lock().map_err(|e| e.to_string())?;
    let project = lib
        .projects
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("Project not found: {}", id))?;

    if let Some(name) = patch.name {
        project.name = name;
    }
    if let Some(description) = patch.description {
        project.description = description;
    }
    if let Some(cover_image) = patch.cover_image {
        project.cover_image = if cover_image.is_empty() {
            None
        } else {
            Some(cover_image)
        };
    }
    if let Some(vp) = patch.viewer_preference {
        project.viewer_preference = if vp.is_empty() { None } else { Some(vp) };
    }
    if let Some(tags) = patch.tags {
        project.tags = tags;
    }
    if let Some(fp) = patch.file_path {
        project.file_missing = !Path::new(&fp).exists();
        project.file_path = fp;
    }

    let updated = project.clone();
    save_library(&lib)?;
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
            e.file_type().is_file()
                && e.file_name()
                    .to_str()
                    .map(|n| n == "project.json")
                    .unwrap_or(false)
        })
        .filter_map(|e| e.path().to_str().map(|s| s.to_string()))
        .collect()
}

// ─── Viewers ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_viewers() -> Vec<Viewer> {
    let base = viewers_base_dir();
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

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn extract_project_name(json: &serde_json::Value) -> Option<String> {
    json.get("title")
        .or_else(|| json.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
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
/// Prod: probes portable and bundled resource layouts (Windows/Linux/macOS).
pub fn viewers_base_dir() -> std::path::PathBuf {
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
    let fresh = reload_from_disk();
    if let Ok(mut lib) = state.lock() {
        *lib = fresh;
    }
}
