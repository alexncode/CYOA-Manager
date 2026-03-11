<script setup lang="ts">
import { computed, onMounted } from "vue";
import { RouterView, RouterLink } from "vue-router";
import { useSettings } from "./composables/useSettings";
import { useLibrary } from "./composables/useLibrary";

const { settings, applyTheme } = useSettings();
const { projects, viewers, loadLibrary, loadViewers, openViewer } = useLibrary();

const randomCandidates = computed(() =>
  projects.value.filter((project) => !project.file_missing)
);

const canOpenRandom = computed(
  () => randomCandidates.value.length > 0 && viewers.value.length > 0
);

onMounted(async () => {
  applyTheme();
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", applyTheme);
  await loadLibrary();
  await loadViewers();
});

async function openRandomProject() {
  if (!canOpenRandom.value) {
    return;
  }

  const candidates = randomCandidates.value;
  const project = candidates[Math.floor(Math.random() * candidates.length)];
  const viewerId = project.viewer_preference
    || settings.value.defaultViewer
    || viewers.value[0]?.id;

  if (!viewerId) {
    return;
  }

  await openViewer(project, viewerId);
}
</script>

<template>
  <div class="app">
    <nav class="sidebar">
      <div class="brand">CYOA<br /><span>Manager</span></div>
      <RouterLink to="/" class="nav-link" active-class="active">
        📚 Library
      </RouterLink>
      <RouterLink to="/settings" class="nav-link" active-class="active">
        ⚙️ Settings
      </RouterLink>
      <RouterLink to="/perks" class="nav-link" active-class="active">
        🔎 All Perks
      </RouterLink>
      <RouterLink to="/catalog" class="nav-link" active-class="active">
        🌐 Infaera Catalog
      </RouterLink>
      <button class="nav-link nav-button" :disabled="!canOpenRandom" @click="openRandomProject">
        🎲 Random
      </button>
      <a
        class="nav-link sidebar-external github-link"
        href="https://github.com/alexncode/CYOA-Manager"
        target="_blank"
        rel="noreferrer"
      >
        VIEW ON GITHUB
      </a>
      <a
        class="nav-link sidebar-external patreon-link"
        href="https://www.patreon.com/interactiveapps"
        target="_blank"
        rel="noreferrer"
      >
        SUPPORT ON PATREON
      </a>
    </nav>
    <main class="main">
      <RouterView />
    </main>
  </div>
</template>

<style>
/* ── CSS Variables ──────────────────────────────────────────── */
:root {
  --bg: #1a1c20;
  --sidebar-bg: #13151a;
  --card-bg: #22252d;
  --dialog-bg: #1e2128;
  --menu-bg: #2a2d36;
  --input-bg: #2a2d36;
  --border: #32363f;
  --text: #e8eaf0;
  --muted: #7a7f92;
  --accent: #5b8af0;
  --accent-hover: #4a79e0;
  --hover: rgba(255, 255, 255, 0.06);
  --tag-bg: rgba(91, 138, 240, 0.18);
  --tag-color: #8bb0ff;
  --cover-placeholder: #2a2d36;
}
:root:not(.dark) {
  --bg: #f3f4f8;
  --sidebar-bg: #e8eaf0;
  --card-bg: #ffffff;
  --dialog-bg: #ffffff;
  --menu-bg: #ffffff;
  --input-bg: #f3f4f8;
  --border: #d4d7e0;
  --text: #1a1c20;
  --muted: #6b7080;
  --accent: #3a6ce8;
  --accent-hover: #2a5cd8;
  --hover: rgba(0, 0, 0, 0.05);
  --tag-bg: rgba(58, 108, 232, 0.12);
  --tag-color: #2a5cd8;
  --cover-placeholder: #d4d7e0;
}

*, *::before, *::after { box-sizing: border-box; }

html, body, #app {
  height: 100%;
  margin: 0;
  padding: 0;
}

body {
  font-family: Inter, system-ui, sans-serif;
  font-size: 15px;
  background: var(--bg);
  color: var(--text);
  -webkit-font-smoothing: antialiased;
  user-select: none;
}

/* ── Shared button styles ──────────────────────────────────── */
.btn-primary {
  background: var(--accent);
  color: #fff;
  border: none;
  border-radius: 8px;
  padding: 7px 16px;
  font-size: 0.875rem;
  font-weight: 600;
  cursor: pointer;
  transition: background 0.15s;
}
.btn-primary:hover:not(:disabled) { background: var(--accent-hover); }
.btn-primary:disabled { opacity: 0.45; cursor: not-allowed; }

.btn-secondary {
  background: transparent;
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 7px 16px;
  font-size: 0.875rem;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s;
}
.btn-secondary:hover { background: var(--hover); }

.btn-ghost {
  background: none;
  border: none;
  color: var(--accent);
  font-size: 0.8rem;
  cursor: pointer;
  padding: 2px 4px;
}
.btn-ghost:hover { text-decoration: underline; }

/* ── Scrollbar ─────────────────────────────────────────────── */
::-webkit-scrollbar { width: 6px; height: 6px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: var(--border); border-radius: 3px; }
</style>

<style scoped>
.app {
  display: flex;
  height: 100vh;
  overflow: hidden;
}
.sidebar {
  width: 180px;
  flex-shrink: 0;
  background: var(--sidebar-bg);
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  padding: 20px 0;
  gap: 4px;
}
.brand {
  font-size: 1.1rem;
  font-weight: 800;
  line-height: 1.2;
  padding: 0 20px 20px;
  color: var(--text);
}
.brand span { color: var(--accent); }
.nav-link {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 20px;
  color: var(--muted);
  text-decoration: none;
  font-size: 0.875rem;
  border-radius: 0;
  transition: background 0.12s, color 0.12s;
}
.nav-link:hover { background: var(--hover); color: var(--text); }
.nav-link.active {
  background: var(--hover);
  color: var(--accent);
  font-weight: 600;
}
.nav-button {
  width: 100%;
  background: none;
  border: none;
  text-align: left;
  font: inherit;
}
.nav-button:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
.nav-button:disabled:hover {
  background: transparent;
  color: var(--muted);
}
.sidebar-external {
  margin-top: auto;
}
.main {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.main :deep(.library-view) {
  flex: 1;
  min-height: 0;
}

.patreon-link {
  background: linear-gradient(90deg, #ff424d, #ff6a6a);
  color: #fff;
  border-radius: 8px;
  padding: 8px 20px;
  font-weight: 600;
  margin: 10px ;
  font-size: 11px;
}

.github-link {
  background: #333;
  color: #fff;
  border-radius: 8px;
  padding: 8px 20px;
  font-weight: 600;
  margin: 0px 10px ;
  font-size: 11px;
}
</style>
