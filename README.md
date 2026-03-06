# CYOA Manager

CYOA Manager is a desktop library app for organizing, previewing, and launching `project.json` based CYOA projects with multiple bundled viewers.

## What It Does

- Adds single projects or bulk-imports folders containing `project.json`
- Detects cover images from project metadata, including relative image paths
- Launches bundled viewers against selected projects
- Stores a local library index and per-project metadata such as tags and viewer preference

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
