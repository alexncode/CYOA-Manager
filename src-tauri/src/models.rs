use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    pub cover_image: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    pub file_path: String,
    pub viewer_preference: Option<String>,
    #[serde(default)]
    pub favorite: bool,
    pub date_added: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub file_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub version: u32,
    pub projects: Vec<Project>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            projects: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    /// `None` → don't touch; `Some("")` → clear; `Some(url)` → set
    pub cover_image: Option<String>,
    pub viewer_preference: Option<String>,
    pub favorite: Option<bool>,
    pub tags: Option<Vec<String>>,
    /// Re-link a broken card to a new path
    pub file_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Viewer {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ViewerSession {
    pub project_id: String,
    pub viewer_id: String,
    pub cheats_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerkIndexStatus {
    pub ready: bool,
    pub needs_reindex: bool,
    pub indexed_projects: usize,
    pub total_projects: usize,
    pub perk_count: usize,
    pub images_enabled: bool,
    pub last_indexed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerkSearchResult {
    pub project_id: String,
    pub project_name: String,
    pub row_id: String,
    pub row_title: String,
    pub object_id: String,
    pub title: String,
    pub description: String,
    pub points: Option<String>,
    pub addons: Vec<String>,
    pub image_path: Option<String>,
}

pub type SessionStore = Mutex<HashMap<String, ViewerSession>>;
