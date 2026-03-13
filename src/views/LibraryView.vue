<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useLibrary } from "../composables/useLibrary";
import { useSettings } from "../composables/useSettings";
import ProjectCard from "../components/ProjectCard.vue";
import AddProjectDialog from "../components/AddProjectDialog.vue";
import BulkImportDialog from "../components/BulkImportDialog.vue";
import DownloadProjectDialog from "../components/DownloadProjectDialog.vue";
import EditProjectDialog from "../components/EditProjectDialog.vue";
import RelinkDialog from "../components/RelinkDialog.vue";
import type { Project, ProjectPatch, SortKey } from "../types";

const {
  projects,
  viewers,
  loading,
  error,
  loadLibrary,
  takeLibraryMigrationNotice,
  removeProject,
  startOverwriteCatalogEntry,
  setProjectFavorite,
  updateProject,
  openViewer,
  allTags,
} = useLibrary();

const { settings } = useSettings();

const search = ref("");
const tagFilter = ref("");
const sort = ref<SortKey>("favorite_date_added");

const showAdd = ref(false);
const showBulk = ref(false);
const showDownload = ref(false);
const editTarget = ref<Project | null>(null);
const relinkTarget = ref<Project | null>(null);
const migrationNotice = ref<string | null>(null);
const redownloadingProjectId = ref<string | null>(null);
const redownloadStatus = ref<string | null>(null);
let redownloadProgressUnlisten: UnlistenFn | null = null;
let activeRedownloadTaskId = "";

type CatalogProgressPayload = {
  taskId: string;
  phase: string;
  current: number;
  total: number;
  message: string;
  done: boolean;
  success: boolean;
  error?: string | null;
};

const displayedList = computed(() => {
  let list = [...projects.value];
  const q = search.value.toLowerCase().trim();
  if (q) {
    list = list.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        p.tags.some((t) => t.toLowerCase().includes(q))
    );
  }
  if (tagFilter.value) {
    list = list.filter((p) => p.tags.includes(tagFilter.value));
  }
  if (sort.value === "name") {
    list.sort((a, b) => a.name.localeCompare(b.name));
  } else if (sort.value === "favorite_date_added") {
    list.sort((a, b) => {
      if (a.favorite !== b.favorite) {
        return Number(b.favorite) - Number(a.favorite);
      }
      return new Date(b.date_added).getTime() - new Date(a.date_added).getTime();
    });
  } else {
    list.sort(
      (a, b) => new Date(b.date_added).getTime() - new Date(a.date_added).getTime()
    );
  }
  return list;
});

onMounted(async () => {
  await loadLibrary();
  migrationNotice.value = await takeLibraryMigrationNotice();
});

async function reloadLibrary() {
  await loadLibrary(true);
}

function closeMigrationNotice() {
  migrationNotice.value = null;
}

async function clearRedownloadListener() {
  if (!redownloadProgressUnlisten) {
    return;
  }

  await redownloadProgressUnlisten();
  redownloadProgressUnlisten = null;
}

async function onRemove(project: Project) {
  if (!confirm(`Remove "${project.name}" from library?`)) return;
  await removeProject(project.id);
}

async function onEdit(project: Project, patch: ProjectPatch) {
  await updateProject(project.id, patch);
  editTarget.value = null;
}

async function onRelink(project: Project, patch: ProjectPatch) {
  await updateProject(project.id, patch);
  relinkTarget.value = null;
}

async function onOpen(project: Project, viewerId: string) {
  await openViewer(project, viewerId);
}

async function onToggleFavorite(project: Project) {
  await setProjectFavorite(project.id, !project.favorite);
}

async function onRedownload(project: Project) {
  if (!project.source_url || !project.source_url.trim()) {
    return;
  }

  if (activeRedownloadTaskId) {
    alert("A re-download is already in progress.");
    return;
  }

  const taskId = crypto.randomUUID();
  activeRedownloadTaskId = taskId;
  redownloadingProjectId.value = project.id;
  redownloadStatus.value = "Preparing...";

  await clearRedownloadListener();

  try {
    redownloadProgressUnlisten = await listen<CatalogProgressPayload>("download-catalog-progress", async (event) => {
      const payload = event.payload;
      if (payload.taskId !== activeRedownloadTaskId) {
        return;
      }

      redownloadStatus.value = payload.message;

      if (!payload.done) {
        return;
      }

      activeRedownloadTaskId = "";
      redownloadingProjectId.value = null;
      await clearRedownloadListener();

      if (payload.success) {
        redownloadStatus.value = null;
        await loadLibrary(true);
        return;
      }

      const message = payload.error || payload.message || "Re-download failed.";
      redownloadStatus.value = null;
      alert(message);
    });

    await startOverwriteCatalogEntry(
      taskId,
      project.id,
      project.source_url,
      "",
      project.name,
      settings.value.downloadSizeLimitMb,
    );
  } catch (redownloadError) {
    activeRedownloadTaskId = "";
    redownloadingProjectId.value = null;
    redownloadStatus.value = null;
    await clearRedownloadListener();
    alert(redownloadError instanceof Error ? redownloadError.message : String(redownloadError));
  }
}

</script>

<template>
  <div class="library-view">
    <!-- Toolbar -->
    <div class="toolbar">
      <input
        v-model="search"
        class="search"
        type="text"
        placeholder="Search projects…"
      />

      <select v-model="tagFilter" class="filter-select" title="Filter by tag">
        <option value="">All tags</option>
        <option v-for="tag in allTags" :key="tag" :value="tag">{{ tag }}</option>
      </select>

      <select v-model="sort" class="filter-select" title="Sort">
        <option value="favorite_date_added">Favorites first</option>
        <option value="date_added">Newest first</option>
        <option value="name">Name A–Z</option>
      </select>

      <div class="project-count">
        {{ projects.length }} {{ projects.length === 1 ? "project" : "projects" }}
      </div>

      <div class="toolbar-spacer" />

      <button class="btn-secondary" @click="showBulk = true">Import folder</button>
      <button class="btn-secondary" @click="showDownload = true">Download Project</button>
      <button class="btn-primary" @click="showAdd = true">+ Add Project</button>
    </div>

    <!-- Loading -->
    <div v-if="loading" class="center-msg">Loading library…</div>

    <!-- Error -->
    <div v-else-if="error" class="center-msg error">{{ error }}</div>

    <!-- Empty state -->
    <div v-else-if="projects.length === 0" class="empty-state">
      <div class="empty-icon">📚</div>
      <h2>Your library is empty</h2>
      <p>Add your first <code>project.json</code> to get started.</p>
      <div class="empty-actions">
        <button class="btn-secondary large" @click="showDownload = true">Download Project</button>
        <button class="btn-primary large" @click="showAdd = true">+ Add Project</button>
      </div>
    </div>

    <!-- No results -->
    <div
      v-else-if="displayedList.length === 0"
      class="center-msg"
    >
      No projects match your filter.
    </div>

    <!-- Card grid -->
    <div v-else class="grid">
      <ProjectCard
        v-for="p in displayedList"
        :key="p.id"
        :project="p"
        :viewers="viewers"
        :default-viewer="settings.defaultViewer"
        :redownload-busy="redownloadingProjectId === p.id"
        :redownload-label="redownloadingProjectId === p.id ? redownloadStatus : null"
        @open="(vid) => onOpen(p, vid)"
        @toggle-favorite="onToggleFavorite(p)"
        @redownload="onRedownload(p)"
        @remove="onRemove(p)"
        @edit="editTarget = p"
        @relink="relinkTarget = p"
      />
    </div>
  </div>

  <!-- Dialogs -->
  <AddProjectDialog
    v-if="showAdd"
    @close="showAdd = false"
    @added="reloadLibrary"
  />
  <BulkImportDialog
    v-if="showBulk"
    @close="showBulk = false"
    @added="reloadLibrary"
  />
  <DownloadProjectDialog
    v-if="showDownload"
    @close="showDownload = false"
    @added="reloadLibrary"
  />
  <EditProjectDialog
    v-if="editTarget"
    :project="editTarget"
    :viewers="viewers"
    @save="(patch) => onEdit(editTarget!, patch)"
    @close="editTarget = null"
  />
  <RelinkDialog
    v-if="relinkTarget"
    :project="relinkTarget"
    @save="(patch) => onRelink(relinkTarget!, patch)"
    @close="relinkTarget = null"
  />

  <div v-if="migrationNotice" class="overlay" @click.self="closeMigrationNotice">
    <div class="dialog migration-dialog">
      <h2>Library Migrated</h2>
      <p>{{ migrationNotice }}</p>
      <div class="dialog-actions">
        <button class="btn-primary" @click="closeMigrationNotice">OK</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.library-view {
  flex: 1;
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  overflow-y: auto;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 20px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
  position: sticky;
  top: 0;
  z-index: 2;
  background: var(--bg);
}

.search {
  flex: 1;
  min-width: 0;
  padding: 7px 12px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--text);
  font-size: 0.9rem;
  outline: none;
  transition: border-color 0.15s;
}
.search:focus { border-color: var(--accent); }

.filter-select {
  padding: 7px 10px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--text);
  font-size: 0.875rem;
  outline: none;
  cursor: pointer;
}

.project-count {
  flex: 0 0 auto;
  padding: 7px 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--muted);
  font-size: 0.875rem;
  white-space: nowrap;
}

.toolbar-spacer { flex: 1; }

.grid {
  flex: none;
  min-height: auto;
  overflow: visible;
  padding: 20px;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 16px;
  align-content: start;
}

.center-msg {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--muted);
  font-size: 0.95rem;
}
.center-msg.error { color: #e55; }

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  color: var(--muted);
  text-align: center;
  padding: 40px;
}
.empty-icon { font-size: 4rem; }
.empty-state h2 { margin: 0; color: var(--text); }
.empty-state p { margin: 0; font-size: 0.95rem; }
.empty-state code { color: var(--accent); }
.empty-actions {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  justify-content: center;
}
.btn-primary.large { padding: 12px 28px; font-size: 1rem; margin-top: 8px; }

.overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 200;
}

.dialog {
  width: min(480px, calc(100vw - 32px));
  background: var(--dialog-bg);
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 22px;
  box-shadow: 0 16px 40px rgba(0, 0, 0, 0.35);
}

.migration-dialog h2 {
  margin: 0 0 10px;
}

.migration-dialog p {
  margin: 0;
  color: var(--muted);
  line-height: 1.5;
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 18px;
}
</style>
