use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    pub cover_image: Option<String>,
    pub file_path: String,
    pub viewer_preference: Option<String>,
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
}

pub type SessionStore = Mutex<HashMap<String, ViewerSession>>;
