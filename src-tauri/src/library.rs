use std::fs;
use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::Library;
use crate::models::Project;

pub struct LibraryLoadResult {
    pub library: Library,
    pub migration_notice: Option<String>,
}

pub fn data_root_dir() -> PathBuf {
    #[cfg(debug_assertions)]
    {
        let manifest = env!("CARGO_MANIFEST_DIR");
        std::path::Path::new(manifest)
            .parent()
            .expect("workspace root not found")
            .to_path_buf()
    }
    #[cfg(not(debug_assertions))]
    {
        if cfg!(target_os = "windows") {
            legacy_data_root_dir()
        } else {
            user_data_root_dir()
        }
    }
}

#[cfg(not(debug_assertions))]
fn legacy_data_root_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("cannot resolve exe path");
    exe.parent().expect("exe has no parent directory").to_path_buf()
}

#[cfg(not(debug_assertions))]
fn user_data_root_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("CYOA Manager");
        }
    }

    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg_data_home).join("cyoa-manager");
    }

    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("cyoa-manager");
    }

    legacy_data_root_dir()
}

pub fn legacy_library_json_path() -> PathBuf {
    data_root_dir().join("save").join("library.json")
}

pub fn legacy_library_backup_path() -> PathBuf {
    data_root_dir().join("save").join("backUplibrary.json")
}

pub fn library_db_path() -> PathBuf {
    data_root_dir().join("save").join("library.sqlite3")
}

pub fn cyoas_dir() -> PathBuf {
    data_root_dir().join("cyoas")
}

pub fn perk_index_db_path() -> PathBuf {
    data_root_dir().join("save").join("perk-index.sqlite3")
}

pub fn perk_images_dir() -> PathBuf {
    data_root_dir().join("save").join("perk-images")
}

pub fn load_library() -> Result<LibraryLoadResult, String> {
    let conn = open_library_connection()?;
    initialize_library_schema(&conn)?;
    let migration_notice = migrate_legacy_library_if_needed(&conn)?;
    let library = read_library_from_db(&conn)?;
    Ok(LibraryLoadResult {
        library,
        migration_notice,
    })
}

pub fn save_library(library: &Library) -> Result<(), String> {
    let mut conn = open_library_connection()?;
    initialize_library_schema(&conn)?;
    write_library_to_db(&mut conn, library)
}

pub fn reload_library() -> Result<Library, String> {
    load_library().map(|result| result.library)
}

fn open_library_connection() -> Result<Connection, String> {
    let path = library_db_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    Connection::open(path).map_err(|e| e.to_string())
}

fn initialize_library_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS library_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS library_projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            cover_image TEXT,
            source_url TEXT,
            file_path TEXT NOT NULL,
            viewer_preference TEXT,
            date_added TEXT NOT NULL,
            tags_json TEXT NOT NULL
        );
        ",
    )
    .map_err(|e| e.to_string())
}

fn read_library_from_db(conn: &Connection) -> Result<Library, String> {
    let version = conn
        .query_row(
            "SELECT value FROM library_meta WHERE key = 'version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1);

    let mut statement = conn
        .prepare(
            "
            SELECT id, name, description, cover_image, source_url, file_path, viewer_preference, date_added, tags_json
            FROM library_projects
            ORDER BY date_added DESC, name COLLATE NOCASE ASC
            ",
        )
        .map_err(|e| e.to_string())?;

    let rows = statement
        .query_map([], |row| {
            let tags_json: String = row.get(8)?;
            let tags = serde_json::from_str(&tags_json).unwrap_or_default();

            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                cover_image: row.get(3)?,
                source_url: row.get(4)?,
                file_path: row.get(5)?,
                viewer_preference: row.get(6)?,
                date_added: row.get(7)?,
                tags,
                file_missing: false,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut projects = Vec::new();
    for row in rows {
        projects.push(row.map_err(|e| e.to_string())?);
    }

    Ok(Library { version, projects })
}

fn write_library_to_db(conn: &mut Connection, library: &Library) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM library_projects", [])
        .map_err(|e| e.to_string())?;

    {
        let mut statement = tx
            .prepare(
                "
                INSERT INTO library_projects (
                    id, name, description, cover_image, source_url, file_path, viewer_preference, date_added, tags_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ",
            )
            .map_err(|e| e.to_string())?;

        for project in &library.projects {
            let tags_json = serde_json::to_string(&project.tags).map_err(|e| e.to_string())?;
            statement
                .execute(params![
                    project.id,
                    project.name,
                    project.description,
                    project.cover_image,
                    project.source_url,
                    project.file_path,
                    project.viewer_preference,
                    project.date_added,
                    tags_json,
                ])
                .map_err(|e| e.to_string())?;
        }
    }

    tx.execute(
        "INSERT INTO library_meta (key, value) VALUES ('version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![library.version.to_string()],
    )
    .map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())
}

fn migrate_legacy_library_if_needed(conn: &Connection) -> Result<Option<String>, String> {
    let legacy_path = legacy_library_json_path();
    if !legacy_path.exists() {
        return Ok(None);
    }

    let project_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM library_projects", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;

    let backup_path = legacy_library_backup_path();

    if project_count == 0 {
        let raw = fs::read_to_string(&legacy_path).map_err(|e| e.to_string())?;
        let legacy_library: Library = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let mut writable = open_library_connection()?;
        initialize_library_schema(&writable)?;
        write_library_to_db(&mut writable, &legacy_library)?;
    }

    if backup_path.exists() {
        fs::remove_file(&backup_path).map_err(|e| e.to_string())?;
    }
    fs::rename(&legacy_path, &backup_path).map_err(|e| e.to_string())?;

    Ok(Some(format!(
        "Your library was migrated from library.json to SQLite. A backup of the old file was saved as {}.",
        backup_path.display()
    )))
}
