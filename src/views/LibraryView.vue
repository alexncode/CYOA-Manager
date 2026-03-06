<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
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
  removeProject,
  updateProject,
  openViewer,
  allTags,
} = useLibrary();

const { settings } = useSettings();

const search = ref("");
const tagFilter = ref("");
const sort = ref<SortKey>("date_added");

const showAdd = ref(false);
const showBulk = ref(false);
const showDownload = ref(false);
const editTarget = ref<Project | null>(null);
const relinkTarget = ref<Project | null>(null);

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
  } else {
    list.sort(
      (a, b) => new Date(b.date_added).getTime() - new Date(a.date_added).getTime()
    );
  }
  return list;
});

onMounted(async () => {
  await loadLibrary();
});

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
        <option value="date_added">Newest first</option>
        <option value="name">Name A–Z</option>
      </select>

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
        @open="(vid) => onOpen(p, vid)"
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
    @added="loadLibrary"
  />
  <BulkImportDialog
    v-if="showBulk"
    @close="showBulk = false"
    @added="loadLibrary"
  />
  <DownloadProjectDialog
    v-if="showDownload"
    @close="showDownload = false"
    @added="loadLibrary"
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
</style>
