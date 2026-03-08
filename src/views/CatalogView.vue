<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CatalogEntry } from "../types";
import { useLibrary } from "../composables/useLibrary";

const REMOTE_CATALOG_URL = "https://infaera.neocities.org/ZipArchive/zip_link_catalog_data.js";
const LOCAL_CATALOG_URL = "/zip_link_catalog_data.js";

const { loadLibrary, startDownloadCatalogEntry } = useLibrary();

const entries = ref<CatalogEntry[]>([]);
const loading = ref(false);
const addingLink = ref<string | null>(null);
const error = ref<string | null>(null);
const sourceLabel = ref("remote");
const search = ref("");
const sort = ref<"name" | "date">("date");
const successMessage = ref<string | null>(null);
const addStatus = ref("");
const addProgress = ref(0);
let catalogProgressUnlisten: UnlistenFn | null = null;
let activeCatalogTaskId = "";

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

type CatalogListEntry = CatalogEntry & {
  siteBadge: string | null;
};

const displayedList = computed(() => {
  let list: CatalogListEntry[] = entries.value.map((entry) => ({
    ...entry,
    siteBadge: extractSiteBadge(entry.website),
  }));
  const query = search.value.trim().toLowerCase();

  if (query) {
    list = list.filter((entry) => {
      return entry.name.toLowerCase().includes(query)
        || entry.website.toLowerCase().includes(query)
        || entry.date.toLowerCase().includes(query)
        || entry.siteBadge?.toLowerCase().includes(query);
    });
  }

  if (sort.value === "name") {
    list.sort((left, right) => left.name.localeCompare(right.name));
  } else {
    list.sort((left, right) => right.date.localeCompare(left.date));
  }

  return list;
});

onMounted(() => {
  void loadCatalog();
});

onBeforeUnmount(() => {
  if (catalogProgressUnlisten) {
    void catalogProgressUnlisten();
    catalogProgressUnlisten = null;
  }
});

async function loadCatalog() {
  loading.value = true;
  error.value = null;
  successMessage.value = null;

  try {
    entries.value = await fetchRemoteCatalogEntries(REMOTE_CATALOG_URL);
    sourceLabel.value = "remote";
  } catch (remoteError) {
    try {
      entries.value = await fetchLocalCatalogEntries(LOCAL_CATALOG_URL);
      sourceLabel.value = "local fallback";
    } catch (localError) {
      const remoteMessage = remoteError instanceof Error ? remoteError.message : String(remoteError);
      const localMessage = localError instanceof Error ? localError.message : String(localError);
      error.value = `Failed to load the catalog. Remote: ${remoteMessage}. Local fallback: ${localMessage}.`;
      entries.value = [];
    }
  } finally {
    loading.value = false;
  }
}

async function addEntry(entry: CatalogEntry) {
  addingLink.value = entry.link;
  error.value = null;
  successMessage.value = null;
  addStatus.value = "Preparing import…";
  addProgress.value = 0;
  const taskId = crypto.randomUUID();

  try {
    if (catalogProgressUnlisten) {
      await catalogProgressUnlisten();
      catalogProgressUnlisten = null;
    }

    activeCatalogTaskId = taskId;
    catalogProgressUnlisten = await listen<CatalogProgressPayload>("download-catalog-progress", async (event) => {
      const payload = event.payload;
      if (!activeCatalogTaskId || payload.taskId !== activeCatalogTaskId) {
        return;
      }

      addStatus.value = payload.message;
      addProgress.value = payload.total > 0 ? payload.current / payload.total : 0;

      if (payload.done) {
        addingLink.value = null;
        activeCatalogTaskId = "";
        if (catalogProgressUnlisten) {
          await catalogProgressUnlisten();
          catalogProgressUnlisten = null;
        }

        if (payload.success) {
          await loadLibrary();
          successMessage.value = payload.message;
        } else {
          error.value = payload.error || "Catalog import failed.";
        }
      }
    });

    await startDownloadCatalogEntry(taskId, entry.website, entry.link, entry.name);
  } catch (addError) {
    addingLink.value = null;
    activeCatalogTaskId = "";
    error.value = addError instanceof Error ? addError.message : String(addError);
  }
}

async function fetchCatalogEntries(url: string): Promise<CatalogEntry[]> {
  const response = await fetch(url, { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }

  const source = await response.text();
  return parseCatalogData(source);
}

async function fetchRemoteCatalogEntries(url: string): Promise<CatalogEntry[]> {
  const source = await invoke<string>("fetch_catalog_source", { url });
  return parseCatalogData(source);
}

async function fetchLocalCatalogEntries(url: string): Promise<CatalogEntry[]> {
  return fetchCatalogEntries(url);
}

function parseCatalogData(source: string): CatalogEntry[] {
  const marker = "window.CATALOG_DATA =";
  const start = source.indexOf(marker);
  if (start === -1) {
    throw new Error("CATALOG_DATA assignment was not found");
  }

  let payload = source.slice(start + marker.length).trim();
  if (payload.endsWith(";")) {
    payload = payload.slice(0, -1);
  }

  const parsed = JSON.parse(payload);
  if (!Array.isArray(parsed)) {
    throw new Error("CATALOG_DATA is not an array");
  }

  const entries = parsed.filter(isCatalogEntry);
  if (entries.length === 0) {
    throw new Error("Catalog did not contain any usable entries");
  }

  return entries;
}

function isCatalogEntry(value: unknown): value is CatalogEntry {
  if (!value || typeof value !== "object") {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return typeof candidate.name === "string"
    && typeof candidate.date === "string"
    && typeof candidate.website === "string"
    && typeof candidate.link === "string";
}

async function copyWebsiteLink(entry: CatalogEntry) {
  try {
    await navigator.clipboard.writeText(entry.website);
    successMessage.value = `Copied link for ${entry.name}.`;
    error.value = null;
  } catch (copyError) {
    error.value = copyError instanceof Error ? copyError.message : String(copyError);
  }
}

function extractSiteBadge(website: string): string | null {
  try {
    const { hostname } = new URL(website);
    const parts = hostname.toLowerCase().split(".").filter(Boolean);
    const lastIndex = parts.length - 1;

    if (parts.length < 3) {
      return null;
    }

    if (parts[lastIndex - 1] === "neocities" && parts[lastIndex] === "org") {
      return formatSiteBadge(parts[0]);
    }

    if (parts[lastIndex - 1] === "nekoweb" && parts[lastIndex] === "org") {
      return formatSiteBadge(parts[0]);
    }

    if (parts[lastIndex - 1] === "github" && parts[lastIndex] === "io") {
      return formatSiteBadge(parts[0]);
    }

    if (parts[0] === "www") {
      return null;
    }

    return formatSiteBadge(parts[0]);
  } catch {
    return null;
  }
}

function formatSiteBadge(value: string): string | null {
  const cleaned = value.trim().replace(/[-_]+/g, " ");
  return cleaned || null;
}

function getSiteBadgeStyle(label: string) {
  const hue = hashLabelToHue(label);

  return {
    backgroundColor: `hsla(${hue}, 72%, 44%, 0.2)`,
    borderColor: `hsla(${hue}, 78%, 58%, 0.42)`,
    color: `hsl(${hue}, 82%, 80%)`,
  };
}

function hashLabelToHue(label: string): number {
  let hash = 0;

  for (const character of label.toLowerCase()) {
    hash = ((hash << 5) - hash) + character.charCodeAt(0);
    hash |= 0;
  }

  return Math.abs(hash) % 360;
}
</script>

<template>
  <div class="catalog-view">
    <div class="toolbar">
      <input
        v-model="search"
        class="search"
        type="text"
        placeholder="CYOA name, author…"
      />

      <select v-model="sort" class="filter-select" title="Sort">
        <option value="date">Newest first</option>
        <option value="name">Name A-Z</option>
      </select>

      <div >All sourced from <b><a style="color:lightblue" href="https://infaera.neocities.org/ZipArchive/" target="_blank" rel="noreferrer">Infaera ZIP CYOA Archive</a></b></div>

      <div class="toolbar-spacer" />

      <button class="btn-secondary" :disabled="loading" @click="loadCatalog">
        {{ loading ? "Loading…" : "Reload Catalog" }}
      </button>
    </div>

    <div class="status-bar">
      <span>Source: {{ sourceLabel }}</span>
      <span>{{ entries.length }} entries</span>
    </div>

    <div v-if="successMessage" class="banner success">{{ successMessage }}</div>
    <div v-if="error" class="banner error">{{ error }}</div>

    <div v-if="loading" class="center-msg">Loading catalog…</div>
    <div v-else-if="displayedList.length === 0" class="center-msg">No catalog entries match your filter.</div>

    <div v-else class="grid">
      <article v-for="entry in displayedList" :key="entry.link" class="card">
        <div class="info">
          <div class="card-head">
            <div class="title-group">
              <h3 class="name" :title="entry.name">{{ entry.name }}</h3>
              <span
                v-if="entry.siteBadge"
                class="badge badge-site"
                :title="`Site badge: ${entry.siteBadge}`"
                :style="getSiteBadgeStyle(entry.siteBadge)"
              >
                {{ entry.siteBadge }}
              </span>
            </div>
            <div class="badge">{{ entry.date }}</div>
          </div>

          <div v-if="addingLink === entry.link" class="progress-wrap">
            <div class="progress-meta">
              <span>{{ addStatus || "Adding…" }}</span>
              <span>{{ Math.round(addProgress * 100) }}%</span>
            </div>
            <div class="progress-bar">
              <div class="progress-fill" :style="{ width: `${Math.round(addProgress * 100)}%` }" />
            </div>
          </div>

          <div class="actions">
            <a class="btn-secondary action-link" :href="entry.website" target="_blank" rel="noreferrer">
              Open Site
            </a>
            <button class="btn-secondary" @click="copyWebsiteLink(entry)">Copy Link</button>
            <button
              class="btn-primary"
              :disabled="addingLink !== null"
              @click="addEntry(entry)"
            >
              {{ addingLink === entry.link ? "Adding…" : "Add to Library" }}
            </button>
          </div>
        </div>
      </article>
    </div>
  </div>
</template>

<style scoped>
.catalog-view {
  flex: 1;
  display: flex;
  flex-direction: column;
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
}

.search:focus,
.filter-select:focus {
  border-color: var(--accent);
}

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

.toolbar-spacer {
  flex: 1;
}

.status-bar {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 20px 0;
  color: var(--muted);
  font-size: 0.8rem;
}

.banner {
  margin: 12px 20px 0;
  padding: 10px 12px;
  border-radius: 8px;
  font-size: 0.85rem;
}

.banner.success {
  background: rgba(95, 179, 107, 0.14);
  border: 1px solid rgba(95, 179, 107, 0.45);
  color: #8ae29a;
}

.banner.error {
  background: rgba(200, 50, 50, 0.15);
  border: 1px solid #c33;
  color: #e88;
}

.grid {
  padding: 20px;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(360px, 1fr));
  gap: 16px;
  align-content: start;
}

.card {
  background: var(--card-bg);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 14px;
  display: flex;
  flex-direction: column;
}

.badge {
  padding: 2px 8px;
  border-radius: 999px;
  background: rgba(0, 0, 0, 0.28);
  border: 1px solid transparent;
  color: #fff;
  font-size: 0.72rem;
  font-weight: 600;
  white-space: nowrap;
}

.badge-site {
  text-transform: none;
}

.info {
  display: flex;
  flex-direction: column;
  gap: 10px;
  flex: 1;
}

.card-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.title-group {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.name {
  margin: 0;
  font-size: 1rem;
  color: var(--text);
}

.actions {
  display: flex;
  gap: 10px;
  margin-top: auto;
}

.progress-wrap {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.progress-meta {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: 0.82rem;
  color: var(--muted);
}

.progress-bar {
  height: 10px;
  border-radius: 999px;
  overflow: hidden;
  background: var(--input-bg);
  border: 1px solid var(--border);
}

.progress-fill {
  height: 100%;
  background: var(--accent);
  transition: width 0.15s ease;
}

.actions > * {
  flex: 1;
}

.action-link {
  text-decoration: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.center-msg {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--muted);
  font-size: 0.95rem;
  padding: 30px;
}
</style>