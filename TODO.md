# CYOA Manager — TODO

## Project Overview

A Tauri (Vue + Rust) desktop app that maintains a library of CYOA's `project.json` files with a card-based UI. When a user opens a project with a viewer, Tauri registers a custom URI protocol that serves the viewer's static assets and intercepts its `project.json` request, substituting the chosen file from the library.

Two bundled viewers located in `/public`:
- `ICC.Plus.Viewer.v2.6.8/` — ICC+ viewer
- `Viewer 1.8/` — Classic viewer

---

## Phase 1 — Library Backend (Rust / Tauri)

### Data Model
- [x] Define `Project` struct: `id`, `name`, `description`, `cover_image` (optional), `file_path`, `viewer_preference`, `date_added`, `tags`
- [x] Define `Library` struct wrapping a `Vec<Project>` with a version field
- [x] Implement JSON (de)serialization via `serde`
- [x] Store library index at a fixed path inside Tauri's app dir (`<exe>/save/library.json`)

### Tauri Commands
- [x] `get_library() -> Vec<Project>` — return all projects
- [x] `add_project(file_path: String) -> Project` — validate, extract metadata, reference the file, return new entry
- [x] `remove_project(id: String)` — remove entry
- [x] `update_project(id: String, patch: ProjectPatch)` — rename, retag, change viewer preference, etc.
- [x] `get_project_json(id: String) -> String` — read and return raw project.json content
- [x] `scan_folder(folder: String) -> Vec<String>` — discover all `project.json` files in a directory tree for bulk import
- [x] `get_viewers() -> Vec<Viewer>` — list viewer folders in `public/` (dev) or `viewers/` (prod)
- [x] `open_viewer_window(...)` — open an independent WebviewWindow with `cyoaview://` URL

### File Storage Strategy
- [x] Decided: **reference-only** (store path, file stays where it is) 
- [x] Detect when a referenced file is missing and mark the card as broken

### Cover Image
- [ ] Open project with viewer and make a screenshot when fully loaded, save to app covers folder
- [x] Extract first image URL from project JSON as auto-cover fallback
- [x] Allow manual cover override stored in library metadata

---

## Phase 2 — Custom Viewer Protocol (Rust)

The viewers are plain HTML/JS apps that perform a relative fetch for `project.json`. The protocol handler serves the viewer assets and intercepts that one request.

- [x] Register custom URI scheme `cyoaview://` in Rust via `register_uri_scheme_protocol`
- [x] Protocol format: `cyoaview://<project-id>/<viewer-id>/...path...`
  - `cyoaview://abc123/icc-plus-viewer-v2-6-8/index.html` → serve `/public/ICC.Plus.Viewer.v2.6.8/index.html`
  - `cyoaview://abc123/icc-plus-viewer-v2-6-8/project.json` → serve the library entry's actual `project.json`
  - `cyoaview://abc123/icc-plus-viewer-v2-6-8/<any other asset>` → serve from the bundled viewer folder
- [x] Implement the Rust handler: parse scheme, slugify folder names → viewer IDs, serve bytes with correct MIME type
- [x] Set `Access-Control-Allow-Origin: *` headers so viewer JS can function normally
- [x] Open a new independent `WebviewWindow` pointed at `cyoaview://<id>/<viewer>/index.html`
- [x] Pass window title as the project name

---

## Phase 3 — Frontend UI (Vue)

### App Shell
- [x] Set up Vue Router with two routes: `/` (library grid) and `/settings`
- [x] Global sidebar layout with app title, Library + Settings nav links

### Library View (`/`)
- [x] Responsive card grid (CSS Grid, min card width 220px)
- [x] **Project Card** component:
  - Cover image (fallback to a deterministic pastel placeholder with project name initials)
  - Project name
  - Tag chips
  - Per-viewer "Open" buttons (one per viewer found in public/)
  - Overflow menu: Edit, Re-link (if missing), Remove
- [x] Empty state with "Add your first project" CTA
- [x] Search bar: filter by name and tags
- [x] Sort: newest first / A–Z

### Add Project Flow
- [x] "Add Project" button → native file picker (via `@tauri-apps/plugin-dialog`) → `add_project` command
- [x] EditProjectDialog: name, description, tags, cover image URL/path override
- [x] Bulk import: "Import folder" → scan folder → checklist → progress counter
- [x] RelinkDialog: pick new file path for broken cards

### Settings (`/settings`)
- [x] Default viewer preference (global fallback)
- [x] Theme: light / dark / system (CSS variables + class toggle)

---

## Phase 4 — Polish & Edge Cases

- [ ] Persist last window size/position via Tauri's `window-state` plugin
- [ ] Handle corrupted / non-CYOA JSON files gracefully (validate schema on import)
- [ ] Drag-and-drop `project.json` files directly onto the library view to add them
- [x] Show a "file missing" badge on cards whose referenced files were deleted
- [x] Allow re-linking a broken card to a new file path
- [ ] Keyboard navigation: arrow keys to move between cards, Enter to open

---

## Phase 5 — Distribution - SKIP

- [ ] Configure Tauri bundler: NSIS installer for Windows, AppImage for Linux, dmg for macOS
- [ ] Add app icons (all sizes required in `src-tauri/icons/`)
- [ ] Set up GitHub Actions CI: lint → test → build for all three platforms
- [ ] Sign the Windows installer (optional, requires a code-signing cert)

---

## Open Questions

- Should the library file live in app data dir or be portable (next to the exe)? - NEXT to the exe
- Should viewer windows be modal/parented to the main window or fully independent? - Independed
- Do the bundled viewers need any patching (e.g. their fetch URL) or does serving `project.json` at the same origin/path work out of the box? - it will work
- Is it worth supporting external / user-added viewers beyond the two bundled ones? - yes, any viewer in their folder should work 
