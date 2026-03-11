use std::fs;
use std::path::PathBuf;

use crate::models::Library;

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

/// Returns the persisted library path inside the app data root.
/// In dev (`cargo run`), this resolves inside the workspace root.
pub fn library_path() -> PathBuf {
    data_root_dir().join("save").join("library.json")
}

pub fn cyoas_dir() -> PathBuf {
    data_root_dir().join("cyoas")
}

pub fn load_library() -> Library {
    let path = library_path();
    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Library::default(),
        }
    } else {
        Library::default()
    }
}

pub fn save_library(library: &Library) -> Result<(), String> {
    let path = library_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let serialized = serde_json::to_string_pretty(library).map_err(|e| e.to_string())?;
    fs::write(&path, serialized).map_err(|e| e.to_string())
}
