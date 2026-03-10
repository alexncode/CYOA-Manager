# CYOA Manager

CYOA Manager is a desktop library app for organizing, previewing, and launching `project.json` based CYOA projects with multiple bundled viewers.

## What It Does

- Builds a local library of `project.json` based CYOA projects from files, folders, direct downloads, and the built-in catalog

## Features

- Add a single `project.json` CYOA file or bulk-import folders containing multiple projects
- Download projects directly from a website or project URL
- Browse the built-in catalog sourced from Google Sheets - https://docs.google.com/spreadsheets/d/1jxBbWB08myhD8YXePPifsWQG3JH2qZtBs9Y5yYcqE7g/
- Search, sort, and filter catalog entries by author, universe, importer, type, PoV, length, tags, and free-text matches
- Support a bulk CYOA download from catalog
- A simple Cheat Menu allowing to modify points or remove requirements in CYOA's

## Stack

- Frontend: Vue 3 + TypeScript + Vite
- Desktop shell: Tauri 2
- Backend: Rust

## Development

Requirements:

- Node.js
- pnpm
- Rust toolchain
- Tauri prerequisites for your platform

Install dependencies:

```bash
pnpm install
```

Run the desktop app in development:

```bash
pnpm tauri dev
```

Build the frontend only:

```bash
pnpm build
```

## Viewer Assets

Bundled viewers live in:

- `public/viewers/ICC Original` - An original Interactive CYOA Creator by MeanDelay
- `public/viewers/ICC2 Plus` - ICC+ by Wahaha303
- `public/viewers/Om1cr0n` - My own interactive creator used for several CYOAs of mine, may not work as it does not have a stable version
