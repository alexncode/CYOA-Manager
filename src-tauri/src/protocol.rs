use std::sync::Mutex;
use std::path::{Component, Path, PathBuf};

use tauri::http::{Request, Response};
use tauri::Manager;

use crate::commands::{slugify, viewers_base_dir};
use crate::models::{Library, SessionStore};

const VIEWER_OVERLAY_SCRIPT_PATH: &str = "__cyoa_manager_viewer_overlay.js";
const VIEWER_OVERLAY_SCRIPT: &str = include_str!("viewer_overlay.js");
const VIEWER_OVERLAY_TEMPLATE_PATH: &str = "__cyoa_manager_viewer_overlay.html";
const VIEWER_OVERLAY_TEMPLATE: &str = include_str!("viewer_overlay.html");

/// Entry point called from `register_uri_scheme_protocol` in lib.rs.
///
/// URL format: `cyoaview://<session-id>/<path>`
///
/// The session-id is a UUID generated when a viewer window is opened.
/// It maps to (project_id, viewer_id) via the SessionStore state.
///
/// All paths, including root-relative ones like `/js/app.js`, resolve
/// correctly because the host is always the session-id, not a path segment.
pub fn handle(app: &tauri::AppHandle, webview_label: &str, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let uri = request.uri();

    // Look up the session by webview label.
    // On Windows, Tauri maps cyoaview://localhost/* → http://cyoaview.localhost/*,
    // so the URI host is always "cyoaview.localhost" — not a session UUID.
    // Using the webview label as the session key works for ALL requests from that
    // viewer window, including root-relative ones (/favicon.ico, /js/app.js).
    let sessions = app.state::<SessionStore>();
    let session = {
        match sessions.lock() {
            Ok(store) => store.get(webview_label).cloned(),
            Err(_) => return err(500, "session store lock poisoned"),
        }
    };
    let Some(session) = session else {
        return err(404, &format!("no session for webview: {}", webview_label));
    };

    let file_path = uri.path().trim_start_matches('/');
    // Normalize empty path to index.html
    let file_path = if file_path.is_empty() { "index.html" } else { file_path };

    if session.cheats_enabled && file_path == VIEWER_OVERLAY_SCRIPT_PATH {
        return javascript(VIEWER_OVERLAY_SCRIPT.as_bytes().to_vec());
    }

    if session.cheats_enabled && file_path == VIEWER_OVERLAY_TEMPLATE_PATH {
        return html(VIEWER_OVERLAY_TEMPLATE.as_bytes().to_vec());
    }

    // Intercept project.json - serve the real file from the library
    if file_path == "project.json" {
        let lib_state = app.state::<Mutex<Library>>();
        return serve_project_json(&session.project_id, &lib_state);
    }

    if let Some(response) = serve_viewer_asset(app, &session.viewer_id, file_path, session.cheats_enabled) {
        return response;
    }

    let lib_state = app.state::<Mutex<Library>>();
    if let Some(response) = serve_project_asset(&session.project_id, file_path, &lib_state) {
        return response;
    }

    err(404, &format!("file not found: {}", file_path))
}


fn serve_project_json(project_id: &str, state: &Mutex<Library>) -> Response<Vec<u8>> {
    let lib = match state.lock() {
        Ok(l) => l,
        Err(_) => return err(500, "library lock poisoned"),
    };
    let Some(project) = lib.projects.iter().find(|p| p.id == project_id) else {
        return err(404, &format!("project not found: {}", project_id));
    };
    match std::fs::read(&project.file_path) {
        Ok(bytes) => Response::builder()
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .status(200)
            .body(bytes)
            .unwrap(),
        Err(e) => err(500, &e.to_string()),
    }
}


fn serve_viewer_asset(
    app: &tauri::AppHandle,
    viewer_id: &str,
    file_path: &str,
    cheats_enabled: bool,
) -> Option<Response<Vec<u8>>> {
    let base = viewers_base_dir(Some(app));

    // Find the viewer folder whose slug matches viewer_id
    let viewer_dir = if let Ok(entries) = std::fs::read_dir(&base) {
        let entries: Vec<_> = entries.flatten().collect();
        entries.into_iter().find(|e| {
            e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && slugify(&e.file_name().to_string_lossy()) == viewer_id
        }).map(|e| e.path())
    } else {
        None
    };

    let Some(viewer_dir) = viewer_dir else {
        return None;
    };

    serve_local_asset(&viewer_dir, file_path, cheats_enabled)
}

fn serve_project_asset(
    project_id: &str,
    file_path: &str,
    state: &Mutex<Library>,
) -> Option<Response<Vec<u8>>> {
    let lib = state.lock().ok()?;
    let project = lib.projects.iter().find(|p| p.id == project_id)?;
    let project_dir = Path::new(&project.file_path).parent()?.to_path_buf();
    serve_local_asset(&project_dir, file_path, false)
}

fn serve_local_asset(base_dir: &Path, file_path: &str, inject_overlay: bool) -> Option<Response<Vec<u8>>> {
    let full_path = safe_join(base_dir, file_path)?;
    let bytes = std::fs::read(&full_path).ok()?;
    let mime = mime_guess::from_path(&full_path)
        .first_raw()
        .unwrap_or("application/octet-stream");

    let bytes = if inject_overlay && mime.eq_ignore_ascii_case("text/html") {
        inject_viewer_overlay(bytes)
    } else {
        bytes
    };

    Some(
        Response::builder()
            .header("Content-Type", mime)
            .header("Access-Control-Allow-Origin", "*")
            .status(200)
            .body(bytes)
            .unwrap(),
    )
}

fn inject_viewer_overlay(bytes: Vec<u8>) -> Vec<u8> {
    let html = String::from_utf8_lossy(&bytes);
    let injection = format!("<script src=\"/{VIEWER_OVERLAY_SCRIPT_PATH}\"></script>");

    if html.contains(VIEWER_OVERLAY_SCRIPT_PATH) {
        return bytes;
    }

    if html.contains("</body>") {
        return html.replacen("</body>", &(injection.clone() + "</body>"), 1).into_bytes();
    }

    if html.contains("</head>") {
        return html.replacen("</head>", &(injection + "</head>"), 1).into_bytes();
    }

    let mut updated = html.into_owned();
    updated.push_str(&injection);
    updated.into_bytes()
}

fn safe_join(base_dir: &Path, file_path: &str) -> Option<PathBuf> {
    let mut result = base_dir.to_path_buf();
    for component in Path::new(file_path).components() {
        match component {
            Component::Normal(part) => result.push(part),
            Component::CurDir => {}
            Component::RootDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    Some(result)
}

// ─── Helper ─────────────────────────────────────────────────────────────────

fn javascript(bytes: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(200)
        .header("Content-Type", "application/javascript; charset=utf-8")
        .header("Access-Control-Allow-Origin", "*")
        .body(bytes)
        .unwrap()
}

fn html(bytes: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(200)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Access-Control-Allow-Origin", "*")
        .body(bytes)
        .unwrap()
}

fn err(status: u16, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(message.as_bytes().to_vec())
        .unwrap()
}
