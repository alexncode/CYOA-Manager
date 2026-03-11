use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::UNIX_EPOCH;

use base64::Engine;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::commands::LibraryState;
use crate::library::{perk_images_dir, perk_index_db_path};
use crate::models::{PerkIndexStatus, PerkSearchResult, Project};

const META_IMAGES_ENABLED: &str = "images_enabled";
const META_LAST_INDEXED_AT: &str = "last_indexed_at";

#[derive(Debug)]
struct IndexedProjectState {
    project_name: String,
    file_signature: String,
}

#[derive(Debug)]
struct ParsedPerk {
    row_id: String,
    row_title: String,
    object_id: String,
    title: String,
    description: String,
    points: Option<String>,
    addons: Vec<String>,
    image_value: Option<String>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PerkIndexProgress {
    task_id: String,
    phase: String,
    current: usize,
    total: usize,
    message: String,
    done: bool,
    success: bool,
    error: Option<String>,
    status: Option<PerkIndexStatus>,
}

#[tauri::command]
pub fn get_perk_index_status(state: State<LibraryState>) -> Result<PerkIndexStatus, String> {
    let library = state.lock().map_err(|error| error.to_string())?;
    compute_status(&library.projects)
}

#[tauri::command]
pub fn sync_perk_index(
    state: State<LibraryState>,
    include_images: bool,
) -> Result<PerkIndexStatus, String> {
    let library = state.lock().map_err(|error| error.to_string())?;
    sync_index(&library.projects, include_images)
}

#[tauri::command]
pub fn rebuild_perk_index(
    state: State<LibraryState>,
    include_images: bool,
) -> Result<PerkIndexStatus, String> {
    let library = state.lock().map_err(|error| error.to_string())?;
    rebuild_index(&library.projects, include_images)
}

#[tauri::command]
pub fn start_perk_index_task(
    app: AppHandle,
    state: State<LibraryState>,
    include_images: bool,
    force_rebuild: bool,
) -> Result<String, String> {
    let projects = state
        .lock()
        .map_err(|error| error.to_string())?
        .projects
        .clone();
    let task_id = Uuid::new_v4().to_string();
    let app_handle = app.clone();
    let task_id_for_thread = task_id.clone();

    thread::spawn(move || {
        emit_perk_index_progress(
            app_handle.clone(),
            PerkIndexProgress {
                task_id: task_id_for_thread.clone(),
                phase: "starting".to_string(),
                current: 0,
                total: 0,
                message: "Preparing perk index".to_string(),
                done: false,
                success: false,
                error: None,
                status: None,
            },
        );

        let result = if force_rebuild {
            rebuild_index_with_report(&projects, include_images, |current, total, message| {
                emit_perk_index_progress(
                    app_handle.clone(),
                    PerkIndexProgress {
                        task_id: task_id_for_thread.clone(),
                        phase: "indexing".to_string(),
                        current,
                        total,
                        message: message.to_string(),
                        done: false,
                        success: false,
                        error: None,
                        status: None,
                    },
                );
            })
        } else {
            sync_index_with_report(&projects, include_images, |current, total, message| {
                emit_perk_index_progress(
                    app_handle.clone(),
                    PerkIndexProgress {
                        task_id: task_id_for_thread.clone(),
                        phase: "indexing".to_string(),
                        current,
                        total,
                        message: message.to_string(),
                        done: false,
                        success: false,
                        error: None,
                        status: None,
                    },
                );
            })
        };

        match result {
            Ok(status) => emit_perk_index_progress(
                app_handle,
                PerkIndexProgress {
                    task_id: task_id_for_thread,
                    phase: "done".to_string(),
                    current: status.indexed_projects,
                    total: status.total_projects,
                    message: format!("Indexed {} perks", status.perk_count),
                    done: true,
                    success: true,
                    error: None,
                    status: Some(status),
                },
            ),
            Err(error) => emit_perk_index_progress(
                app_handle,
                PerkIndexProgress {
                    task_id: task_id_for_thread,
                    phase: "error".to_string(),
                    current: 0,
                    total: 0,
                    message: "Perk index failed".to_string(),
                    done: true,
                    success: false,
                    error: Some(error),
                    status: None,
                },
            ),
        }
    });

    Ok(task_id)
}

#[tauri::command]
pub fn search_perks(
    query: String,
    limit: usize,
    offset: usize,
) -> Result<Vec<PerkSearchResult>, String> {
    let db_path = perk_index_db_path();
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let conn = open_connection()?;
    let limit = limit.clamp(1, 200);
    let offset = offset.min(50_000);
    let trimmed = query.trim();

    if trimmed.is_empty() {
        let mut statement = conn
            .prepare(
                "SELECT project_id, project_name, row_id, row_title, object_id, title, description, points, addons_json, image_path
                 FROM perks
                 ORDER BY project_name COLLATE NOCASE, row_title COLLATE NOCASE, title COLLATE NOCASE
                 LIMIT ?1 OFFSET ?2",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map(params![limit as i64, offset as i64], map_perk_row)
            .map_err(|error| error.to_string())?;

        return collect_rows(rows);
    }

    let fts_query = build_fts_query(trimmed);
    let mut statement = conn
        .prepare(
            "SELECT p.project_id, p.project_name, p.row_id, p.row_title, p.object_id, p.title, p.description, p.points, p.addons_json, p.image_path
             FROM perks_fts
             JOIN perks p ON p.id = perks_fts.rowid
             WHERE perks_fts MATCH ?1
             ORDER BY bm25(perks_fts), p.project_name COLLATE NOCASE, p.row_title COLLATE NOCASE, p.title COLLATE NOCASE
             LIMIT ?2 OFFSET ?3",
        )
        .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map(params![fts_query, limit as i64, offset as i64], map_perk_row)
        .map_err(|error| error.to_string())?;

    collect_rows(rows)
}

pub fn sync_index_for_project_if_present(project: &Project) -> Result<(), String> {
    let db_path = perk_index_db_path();
    if !db_path.exists() {
        return Ok(());
    }

    if !Path::new(&project.file_path).exists() {
        return remove_project_from_index_if_present(&project.id);
    }

    let mut conn = open_connection()?;
    let images_enabled = get_meta_bool(&conn, META_IMAGES_ENABLED)?.unwrap_or(false);
    index_single_project(&mut conn, project, images_enabled)?;
    set_meta(&conn, META_LAST_INDEXED_AT, &Utc::now().to_rfc3339())?;
    Ok(())
}

pub fn remove_project_from_index_if_present(project_id: &str) -> Result<(), String> {
    let db_path = perk_index_db_path();
    if !db_path.exists() {
        return Ok(());
    }

    let mut conn = open_connection()?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    delete_project_rows(&tx, project_id)?;
    tx.commit().map_err(|error| error.to_string())?;
    set_meta(&conn, META_LAST_INDEXED_AT, &Utc::now().to_rfc3339())?;
    Ok(())
}

pub fn clear_index_if_present() -> Result<(), String> {
    let db_path = perk_index_db_path();
    if !db_path.exists() {
        return Ok(());
    }

    let mut conn = open_connection()?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    clear_tables(&tx)?;
    tx.commit().map_err(|error| error.to_string())?;
    if perk_images_dir().exists() {
        fs::remove_dir_all(perk_images_dir()).map_err(|error| error.to_string())?;
    }
    set_meta(&conn, META_LAST_INDEXED_AT, &Utc::now().to_rfc3339())?;
    Ok(())
}

fn compute_status(projects: &[Project]) -> Result<PerkIndexStatus, String> {
    let indexable_projects = collect_indexable_projects(projects);
    let db_path = perk_index_db_path();
    if !db_path.exists() {
        return Ok(PerkIndexStatus {
            ready: false,
            needs_reindex: !indexable_projects.is_empty(),
            indexed_projects: 0,
            total_projects: indexable_projects.len(),
            perk_count: 0,
            images_enabled: false,
            last_indexed_at: None,
        });
    }

    let conn = open_connection()?;
    create_schema(&conn)?;
    build_status(&conn, &indexable_projects)
}

fn sync_index(projects: &[Project], include_images: bool) -> Result<PerkIndexStatus, String> {
    sync_index_with_report(projects, include_images, |_, _, _| {})
}

fn sync_index_with_report<F>(
    projects: &[Project],
    include_images: bool,
    mut report: F,
) -> Result<PerkIndexStatus, String>
where
    F: FnMut(usize, usize, &str),
{
    let indexable_projects = collect_indexable_projects(projects);
    let mut conn = open_connection()?;
    create_schema(&conn)?;

    let current_images_enabled = get_meta_bool(&conn, META_IMAGES_ENABLED)?.unwrap_or(false);
    if current_images_enabled != include_images {
        return rebuild_index_with_report(projects, include_images, report);
    }

    let indexed = load_indexed_projects(&conn)?;
    let library_ids = indexable_projects
        .iter()
        .map(|project| project.id.clone())
        .collect::<HashSet<_>>();
    let total = indexable_projects.len();
    report(0, total, "Scanning projects");

    let tx = conn.transaction().map_err(|error| error.to_string())?;
    for project_id in indexed.keys() {
        if !library_ids.contains(project_id) {
            delete_project_rows(&tx, project_id)?;
        }
    }

    for (index, project) in indexable_projects.iter().enumerate() {
        let signature = project_signature(&project.file_path)?;
        let needs_update = indexed
            .get(&project.id)
            .map(|entry| entry.file_signature != signature || entry.project_name != project.name)
            .unwrap_or(true);

        if needs_update {
            report(index, total, &format!("Indexing {}", project.name));
            index_project(&tx, project, include_images, &signature)?;
        }
    }

    tx.commit().map_err(|error| error.to_string())?;
    set_meta(&conn, META_IMAGES_ENABLED, if include_images { "1" } else { "0" })?;
    set_meta(&conn, META_LAST_INDEXED_AT, &Utc::now().to_rfc3339())?;
    report(total, total, "Finalizing index");
    build_status(&conn, &indexable_projects)
}

fn rebuild_index(projects: &[Project], include_images: bool) -> Result<PerkIndexStatus, String> {
    rebuild_index_with_report(projects, include_images, |_, _, _| {})
}

fn rebuild_index_with_report<F>(
    projects: &[Project],
    include_images: bool,
    mut report: F,
) -> Result<PerkIndexStatus, String>
where
    F: FnMut(usize, usize, &str),
{
    let indexable_projects = collect_indexable_projects(projects);
    let mut conn = open_connection()?;
    create_schema(&conn)?;

    let tx = conn.transaction().map_err(|error| error.to_string())?;
    clear_tables(&tx)?;
    tx.commit().map_err(|error| error.to_string())?;

    if perk_images_dir().exists() {
        fs::remove_dir_all(perk_images_dir()).map_err(|error| error.to_string())?;
    }

    let total = indexable_projects.len();
    report(0, total, "Rebuilding perk index");

    let tx = conn.transaction().map_err(|error| error.to_string())?;
    for (index, project) in indexable_projects.iter().enumerate() {
        report(index, total, &format!("Indexing {}", project.name));
        let signature = project_signature(&project.file_path)?;
        index_project(&tx, project, include_images, &signature)?;
    }
    tx.commit().map_err(|error| error.to_string())?;

    set_meta(&conn, META_IMAGES_ENABLED, if include_images { "1" } else { "0" })?;
    set_meta(&conn, META_LAST_INDEXED_AT, &Utc::now().to_rfc3339())?;
    report(total, total, "Finalizing index");
    build_status(&conn, &indexable_projects)
}

fn index_single_project(
    conn: &mut Connection,
    project: &Project,
    include_images: bool,
) -> Result<(), String> {
    let signature = project_signature(&project.file_path)?;
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    index_project(&tx, project, include_images, &signature)?;
    tx.commit().map_err(|error| error.to_string())
}

fn index_project(
    tx: &Transaction<'_>,
    project: &Project,
    include_images: bool,
    signature: &str,
) -> Result<(), String> {
    delete_project_rows(tx, &project.id)?;

    let content = fs::read_to_string(&project.file_path).map_err(|error| error.to_string())?;
    let json: Value = serde_json::from_str(&content).map_err(|error| error.to_string())?;
    let perks = extract_perks(&json);

    if include_images {
        let project_dir = perk_images_dir().join(&project.id);
        if project_dir.exists() {
            fs::remove_dir_all(&project_dir).map_err(|error| error.to_string())?;
        }
    }

    for (index, perk) in perks.iter().enumerate() {
        let image_path = if include_images {
            extract_image(project, perk, index)?
        } else {
            None
        };

        let addons_json = serde_json::to_string(&perk.addons).map_err(|error| error.to_string())?;
        let addons_text = perk.addons.join(" \n ");

        tx.execute(
            "INSERT INTO perks (
                project_id, project_name, row_id, row_title, object_id, title, description, points, addons_json, image_path
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                project.id,
                project.name,
                perk.row_id,
                perk.row_title,
                perk.object_id,
                perk.title,
                perk.description,
                perk.points,
                addons_json,
                image_path,
            ],
        )
        .map_err(|error| error.to_string())?;

        let row_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO perks_fts (rowid, title, description, addons_text, row_title)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                row_id,
                perk.title,
                perk.description,
                addons_text,
                perk.row_title,
            ],
        )
        .map_err(|error| error.to_string())?;
    }

    tx.execute(
        "INSERT INTO indexed_projects (project_id, project_name, file_path, file_signature, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(project_id) DO UPDATE SET
            project_name = excluded.project_name,
            file_path = excluded.file_path,
            file_signature = excluded.file_signature,
            indexed_at = excluded.indexed_at",
        params![
            project.id,
            project.name,
            project.file_path,
            signature,
            Utc::now().to_rfc3339(),
        ],
    )
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn build_status(conn: &Connection, projects: &[Project]) -> Result<PerkIndexStatus, String> {
    let indexed = load_indexed_projects(conn)?;
    let images_enabled = get_meta_bool(conn, META_IMAGES_ENABLED)?.unwrap_or(false);
    let last_indexed_at = get_meta(conn, META_LAST_INDEXED_AT)?;
    let perk_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM perks", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;

    let mut needs_reindex = indexed.len() != projects.len();
    if !needs_reindex {
        for project in projects {
            let signature = project_signature(&project.file_path)?;
            let Some(entry) = indexed.get(&project.id) else {
                needs_reindex = true;
                break;
            };

            if entry.file_signature != signature || entry.project_name != project.name {
                needs_reindex = true;
                break;
            }
        }
    }

    Ok(PerkIndexStatus {
        ready: !needs_reindex && indexed.len() == projects.len(),
        needs_reindex,
        indexed_projects: indexed.len(),
        total_projects: projects.len(),
        perk_count: perk_count as usize,
        images_enabled,
        last_indexed_at,
    })
}

fn open_connection() -> Result<Connection, String> {
    let path = perk_index_db_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    create_schema(&conn)?;
    Ok(conn)
}

fn create_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS indexed_projects (
            project_id TEXT PRIMARY KEY,
            project_name TEXT NOT NULL,
            file_path TEXT NOT NULL,
            file_signature TEXT NOT NULL,
            indexed_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS perks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_id TEXT NOT NULL,
            project_name TEXT NOT NULL,
            row_id TEXT NOT NULL,
            row_title TEXT NOT NULL,
            object_id TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT NOT NULL,
            points TEXT,
            addons_json TEXT NOT NULL,
            image_path TEXT
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS perks_fts USING fts5(
            title,
            description,
            addons_text,
            row_title,
            tokenize = 'unicode61'
        );",
    )
    .map_err(|error| error.to_string())
}

fn collect_indexable_projects(projects: &[Project]) -> Vec<Project> {
    projects
        .iter()
        .filter(|project| Path::new(&project.file_path).exists())
        .cloned()
        .collect()
}

fn load_indexed_projects(conn: &Connection) -> Result<HashMap<String, IndexedProjectState>, String> {
    let mut statement = conn
        .prepare("SELECT project_id, project_name, file_signature FROM indexed_projects")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                IndexedProjectState {
                    project_name: row.get(1)?,
                    file_signature: row.get(2)?,
                },
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut indexed = HashMap::new();
    for row in rows {
        let (project_id, state) = row.map_err(|error| error.to_string())?;
        indexed.insert(project_id, state);
    }
    Ok(indexed)
}

fn clear_tables(tx: &Transaction<'_>) -> Result<(), String> {
    tx.execute("DELETE FROM perks_fts", [])
        .map_err(|error| error.to_string())?;
    tx.execute("DELETE FROM perks", [])
        .map_err(|error| error.to_string())?;
    tx.execute("DELETE FROM indexed_projects", [])
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn delete_project_rows(tx: &Transaction<'_>, project_id: &str) -> Result<(), String> {
    let mut statement = tx
        .prepare("SELECT id FROM perks WHERE project_id = ?1")
        .map_err(|error| error.to_string())?;
    let ids = statement
        .query_map(params![project_id], |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?;

    for id in ids {
        tx.execute(
            "DELETE FROM perks_fts WHERE rowid = ?1",
            params![id.map_err(|error| error.to_string())?],
        )
        .map_err(|error| error.to_string())?;
    }

    tx.execute("DELETE FROM perks WHERE project_id = ?1", params![project_id])
        .map_err(|error| error.to_string())?;
    tx.execute(
        "DELETE FROM indexed_projects WHERE project_id = ?1",
        params![project_id],
    )
    .map_err(|error| error.to_string())?;

    let project_image_dir = perk_images_dir().join(project_id);
    if project_image_dir.exists() {
        fs::remove_dir_all(project_image_dir).map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |row| row.get(0))
        .optional()
        .map_err(|error| error.to_string())
}

fn get_meta_bool(conn: &Connection, key: &str) -> Result<Option<bool>, String> {
    Ok(get_meta(conn, key)?.map(|value| value == "1"))
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn extract_perks(json: &Value) -> Vec<ParsedPerk> {
    let Some(rows) = json.get("rows").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut perks = Vec::new();
    for row in rows {
        let row_id = row
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let row_title = row
            .get("title")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| row_id.as_str())
            .to_string();

        let Some(objects) = row.get("objects").and_then(Value::as_array) else {
            continue;
        };

        for object in objects {
            let object_id = object
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let title = object
                .get("title")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| object_id.as_str())
                .to_string();
            let description = object
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let points = format_points(object.get("scores").and_then(Value::as_array));
            let addons = flatten_addons(object.get("addons"));
            let image_value = object
                .get("image")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);

            if title.is_empty() && description.is_empty() && addons.is_empty() {
                continue;
            }

            perks.push(ParsedPerk {
                row_id: row_id.clone(),
                row_title: row_title.clone(),
                object_id,
                title,
                description,
                points,
                addons,
                image_value,
            });
        }
    }

    perks
}

fn format_points(scores: Option<&Vec<Value>>) -> Option<String> {
    let first = scores.and_then(|items| items.first())?;
    let before = first
        .get("beforeText")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let after = first
        .get("afterText")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let value = first.get("value").map(value_to_string).unwrap_or_default();

    let mut parts = Vec::new();
    if !before.is_empty() {
        parts.push(before.to_string());
    }
    if !value.is_empty() {
        parts.push(value);
    }
    if !after.is_empty() {
        parts.push(after.to_string());
    }

    let combined = parts.join(" ").trim().to_string();
    if combined.is_empty() {
        None
    } else {
        Some(combined)
    }
}

fn flatten_addons(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };

    let mut flattened = Vec::new();
    for item in items {
        collect_addon_fragments(item, &mut flattened);
    }
    flattened.retain(|value| !value.trim().is_empty());
    flattened
}

fn collect_addon_fragments(value: &Value, flattened: &mut Vec<String>) {
    match value {
        Value::String(text) => flattened.push(text.trim().to_string()),
        Value::Number(number) => flattened.push(number.to_string()),
        Value::Bool(flag) => flattened.push(flag.to_string()),
        Value::Array(items) => {
            for item in items {
                collect_addon_fragments(item, flattened);
            }
        }
        Value::Object(map) => {
            let mut parts = Vec::new();
            for key in ["title", "text", "description", "label", "name", "id", "value"] {
                if let Some(entry) = map.get(key) {
                    match entry {
                        Value::String(text) if !text.trim().is_empty() => {
                            parts.push(text.trim().to_string())
                        }
                        Value::Number(number) => parts.push(number.to_string()),
                        _ => {}
                    }
                }
            }

            if !parts.is_empty() {
                flattened.push(parts.join(" - "));
            } else {
                for entry in map.values() {
                    collect_addon_fragments(entry, flattened);
                }
            }
        }
        Value::Null => {}
    }
}

fn extract_image(project: &Project, perk: &ParsedPerk, index: usize) -> Result<Option<String>, String> {
    let Some(image_value) = perk.image_value.as_deref() else {
        return Ok(None);
    };

    let project_dir = perk_images_dir().join(&project.id);
    fs::create_dir_all(&project_dir).map_err(|error| error.to_string())?;
    let base_name = format!(
        "{}-{}-{}",
        sanitize_segment(&perk.row_id),
        sanitize_segment(&perk.object_id),
        index
    );

    if let Some((mime_type, encoded)) = image_value.split_once(',') {
        if mime_type.starts_with("data:") {
            let extension = mime_type
                .split(';')
                .next()
                .and_then(|value| value.split('/').nth(1))
                .unwrap_or("bin");
            let file_path = project_dir.join(format!("{}.{}", base_name, normalize_extension(extension)));
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| error.to_string())?;
            fs::write(&file_path, bytes).map_err(|error| error.to_string())?;
            return Ok(Some(file_path.to_string_lossy().to_string()));
        }
    }

    if image_value.starts_with("http://") || image_value.starts_with("https://") {
        return Ok(None);
    }

    let source_path = resolve_local_image_path(project, image_value);
    if !source_path.exists() {
        return Ok(None);
    }

    let extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("img");
    let file_path = project_dir.join(format!("{}.{}", base_name, normalize_extension(extension)));
    fs::copy(source_path, &file_path).map_err(|error| error.to_string())?;
    Ok(Some(file_path.to_string_lossy().to_string()))
}

fn resolve_local_image_path(project: &Project, image_value: &str) -> PathBuf {
    let image_path = Path::new(image_value);
    if image_path.is_absolute() {
        return image_path.to_path_buf();
    }

    let project_path = Path::new(&project.file_path);
    project_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(image_path)
}

fn project_signature(file_path: &str) -> Result<String, String> {
    let metadata = fs::metadata(file_path).map_err(|error| error.to_string())?;
    let length = metadata.len();
    let modified = metadata
        .modified()
        .map_err(|error| error.to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    Ok(format!("{}-{}", length, modified))
}

fn build_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn map_perk_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PerkSearchResult> {
    let addons_json: String = row.get(8)?;
    let addons = serde_json::from_str(&addons_json).unwrap_or_default();
    Ok(PerkSearchResult {
        project_id: row.get(0)?,
        project_name: row.get(1)?,
        row_id: row.get(2)?,
        row_title: row.get(3)?,
        object_id: row.get(4)?,
        title: row.get(5)?,
        description: row.get(6)?,
        points: row.get(7)?,
        addons,
        image_path: row.get(9)?,
    })
}

fn collect_rows(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<PerkSearchResult>>,
) -> Result<Vec<PerkSearchResult>, String> {
    let mut results = Vec::new();
    for row in rows {
        results.push(row.map_err(|error| error.to_string())?);
    }
    Ok(results)
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => String::new(),
    }
}

fn normalize_extension(extension: &str) -> String {
    match extension.trim().to_ascii_lowercase().as_str() {
        "jpeg" => "jpg".to_string(),
        "svg+xml" => "svg".to_string(),
        "" => "img".to_string(),
        other => other.to_string(),
    }
}

fn sanitize_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    sanitized.trim_matches('-').to_string().chars().take(48).collect()
}

fn emit_perk_index_progress(app: AppHandle, payload: PerkIndexProgress) {
    let _ = app.emit("perk-index-progress", payload);
}