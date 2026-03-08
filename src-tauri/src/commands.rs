use std::collections::{HashMap, HashSet};
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

    let mut parsed = tauri::Url::parse(url.trim()).map_err(|e| format!("Invalid URL: {}", e))?;
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

    parsed = resolve_project_json_url(parsed)?;

    let response = reqwest::blocking::get(parsed.clone())
        .map_err(|e| format!("Download failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {}", e))?;

    let bytes = response
        .bytes()
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    let processed = inline_downloaded_project_images(bytes.as_ref(), &parsed, app, task_id)?;

    let dir = cyoas_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create cyoas folder: {}", e))?;

    let destination = unique_path(dir.join(download_file_name(&parsed)));
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

fn resolve_project_json_url(mut url: tauri::Url) -> Result<tauri::Url, String> {
    if url.path().to_ascii_lowercase().ends_with("project.json") {
        return Ok(url);
    }

    if is_cyoa_cafe_host(url.host_str()) {
        if let Some(game_id) = extract_cyoa_cafe_game_id(&url) {
            if let Some(source_url) = fetch_cyoa_cafe_game_source_url(&game_id)? {
                let parsed_source = tauri::Url::parse(source_url.trim())
                    .map_err(|e| format!("Invalid source URL from cyoa.cafe: {}", e))?;
                return resolve_project_json_url(parsed_source);
            }
        }

        let response = reqwest::blocking::get(url.clone())
            .map_err(|e| format!("Failed to open cyoa.cafe page: {}", e))?
            .error_for_status()
            .map_err(|e| format!("Failed to open cyoa.cafe page: {}", e))?;
        let final_url = tauri::Url::parse(response.url().as_str())
            .map_err(|e| format!("Failed to parse cyoa.cafe URL: {}", e))?;
        let html = response
            .text()
            .map_err(|e| format!("Failed to read cyoa.cafe page: {}", e))?;
        if let Some(project_url) = find_project_json_url_in_html(&html, &final_url) {
            return Ok(project_url);
        }

        return Err("Could not find a project.json link on the provided cyoa.cafe page".to_string());
    }

    let mut path = url.path().trim_end_matches('/').to_string();
    if path.is_empty() {
        path = "/project.json".to_string();
    } else {
        path.push_str("/project.json");
    }
    url.set_path(&path);
    Ok(url)
}

fn is_cyoa_cafe_host(host: Option<&str>) -> bool {
    host.map(|value| {
        let lower = value.to_ascii_lowercase();
        lower == "cyoa.cafe" || lower.ends_with(".cyoa.cafe")
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

    if parsed.path().to_ascii_lowercase().ends_with("project.json") {
        Some(parsed)
    } else {
        None
    }
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

    let project = Project {
        id: Uuid::new_v4().to_string(),
        name,
        description: String::new(),
        cover_image,
        file_path,
        viewer_preference: None,
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
