<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CatalogEntry } from "../types";
import { useLibrary } from "../composables/useLibrary";

const GOOGLE_SHEETS_API_KEY = "AIzaSyBRhMRxRwP23DhvQdjCuk1saB5q2Xnp2kk";
const GOOGLE_SHEETS_SPREADSHEET_ID = "1jxBbWB08myhD8YXePPifsWQG3JH2qZtBs9Y5yYcqE7g";
const GOOGLE_SHEETS_SHEET_NAME = "Beta Index";
const LOCAL_CATALOG_URL = "/zip_link_catalog_data.js";

const { loadLibrary, startDownloadCatalogEntry } = useLibrary();

const entries = ref<CatalogEntry[]>([]);
const loading = ref(false);
const addingLink = ref<string | null>(null);
const error = ref<string | null>(null);
const sourceLabel = ref("remote");
const search = ref("");
const sort = ref<"name" | "date">("date");
const authorFilter = ref("");
const universeFilter = ref("");
const importerFilter = ref("");
const typeFilter = ref("");
const povFilter = ref("");
const lengthFilter = ref("");
const tagFilter = ref("");
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
  catalogKey: string;
  hostLabel: string | null;
};

type GoogleSheetResponse = {
  values?: string[][];
};

const displayedList = computed(() => {
  let list: CatalogListEntry[] = entries.value.map((entry) => ({
    ...entry,
    catalogKey: buildCatalogKey(entry),
    hostLabel: extractHostLabel(entry.website),
  }));
  const query = search.value.trim().toLowerCase();

  if (query) {
    list = list.filter((entry) => {
      return [
        entry.name,
        entry.website,
        entry.date,
        entry.author,
        entry.universe,
        entry.importer,
        entry.type,
        entry.pov,
        entry.length,
        entry.description,
        entry.hostLabel,
        ...(entry.tags || []),
      ]
        .filter((value): value is string => Boolean(value))
        .some((value) => value.toLowerCase().includes(query));
    });
  }

  if (authorFilter.value) {
    list = list.filter((entry) => entry.author === authorFilter.value);
  }

  if (universeFilter.value) {
    list = list.filter((entry) => entry.universe === universeFilter.value);
  }

  if (importerFilter.value) {
    list = list.filter((entry) => entry.importer === importerFilter.value);
  }

  if (typeFilter.value) {
    list = list.filter((entry) => entry.type === typeFilter.value);
  }

  if (povFilter.value) {
    list = list.filter((entry) => entry.pov === povFilter.value);
  }

  if (lengthFilter.value) {
    list = list.filter((entry) => entry.length === lengthFilter.value);
  }

  if (tagFilter.value) {
    list = list.filter((entry) => entry.tags?.includes(tagFilter.value));
  }

  if (sort.value === "name") {
    list.sort((left, right) => left.name.localeCompare(right.name));
  } else {
    list.sort((left, right) => right.date.localeCompare(left.date));
  }

  return list;
});

const authorOptions = computed(() => buildOptionList(entries.value.map((entry) => entry.author)));
const universeOptions = computed(() => buildOptionList(entries.value.map((entry) => entry.universe)));
const importerOptions = computed(() => buildOptionList(entries.value.map((entry) => entry.importer)));
const typeOptions = computed(() => buildOptionList(entries.value.map((entry) => entry.type)));
const povOptions = computed(() => buildOptionList(entries.value.map((entry) => entry.pov)));
const lengthOptions = computed(() => buildOptionList(entries.value.map((entry) => entry.length)));
const tagOptions = computed(() => buildOptionList(entries.value.flatMap((entry) => entry.tags || [])));

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
    const zipFallbacks = await loadZipFallbackMap();
    entries.value = await fetchGoogleSheetCatalogEntries(zipFallbacks);
    sourceLabel.value = zipFallbacks.size > 0
      ? "Google Sheets + local ZIP fallback"
      : "Google Sheets";
  } catch (catalogError) {
    error.value = catalogError instanceof Error ? catalogError.message : String(catalogError);
    entries.value = [];
  } finally {
    loading.value = false;
  }
}

async function addEntry(entry: CatalogEntry) {
  addingLink.value = buildCatalogKey(entry);
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

async function loadZipFallbackMap(): Promise<Map<string, CatalogEntry>> {
  const fallbackEntries = await fetchLocalCatalogEntries(LOCAL_CATALOG_URL);
  const map = new Map<string, CatalogEntry>();

  for (const entry of fallbackEntries) {
    const key = normalizeCatalogWebsite(entry.website);
    const current = map.get(key);
    if (!current || compareCatalogDates(entry.date, current.date) > 0) {
      map.set(key, entry);
    }
  }

  return map;
}

async function fetchGoogleSheetCatalogEntries(zipFallbacks: Map<string, CatalogEntry>): Promise<CatalogEntry[]> {
  const url = buildGoogleSheetApiUrl();
  const response = await fetch(url, { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`Failed to load Google Sheets catalog: HTTP ${response.status}`);
  }

  const payload = await response.json() as GoogleSheetResponse;
  const values = payload.values;
  if (!Array.isArray(values) || values.length < 2) {
    throw new Error("Google Sheets catalog did not contain any rows");
  }

  return mapGoogleSheetRowsToCatalogEntries(values, zipFallbacks);
}

async function fetchLocalCatalogEntries(url: string): Promise<CatalogEntry[]> {
  return fetchCatalogEntries(url);
}

function buildGoogleSheetApiUrl(): string {
  const encodedSheetName = encodeURIComponent(GOOGLE_SHEETS_SHEET_NAME);
  return `https://sheets.googleapis.com/v4/spreadsheets/${GOOGLE_SHEETS_SPREADSHEET_ID}/values/${encodedSheetName}?key=${GOOGLE_SHEETS_API_KEY}`;
}

function mapGoogleSheetRowsToCatalogEntries(rows: string[][], zipFallbacks: Map<string, CatalogEntry>): CatalogEntry[] {
  const [headerRow, ...dataRows] = rows;
  const columns = buildGoogleSheetColumnIndex(headerRow);
  const entries: CatalogEntry[] = [];

  for (const row of dataRows) {
    const name = readGoogleSheetCell(row, columns, "Title");
    const website = readGoogleSheetCell(row, columns, "Interactive");
    if (!name || !website) {
      continue;
    }

    const date = readGoogleSheetCell(row, columns, "Updated")
      || readGoogleSheetCell(row, columns, "Added")
      || readGoogleSheetCell(row, columns, "Posted");
    const archiveMatch = zipFallbacks.get(normalizeCatalogWebsite(website));

    entries.push({
      name,
      date: date || "",
      website,
      link: archiveMatch?.link || "",
      author: readGoogleSheetCell(row, columns, "Author"),
      universe: readGoogleSheetCell(row, columns, "Universe"),
      importer: readGoogleSheetCell(row, columns, "Importer"),
      type: readGoogleSheetCell(row, columns, "Type"),
      pov: readGoogleSheetCell(row, columns, "POV"),
      length: readGoogleSheetCell(row, columns, "Length"),
      tags: combineCatalogTags(
        readGoogleSheetCell(row, columns, "Design Tags"),
        readGoogleSheetCell(row, columns, "Content Tags"),
      ),
      description: readGoogleSheetCell(row, columns, "Description"),
    });
  }

  if (entries.length === 0) {
    throw new Error("Google Sheets catalog did not contain any usable entries");
  }

  return entries;
}

function buildGoogleSheetColumnIndex(headerRow: string[]): Map<string, number> {
  return new Map(headerRow.map((header, index) => [header.trim(), index]));
}

function readGoogleSheetCell(row: string[], columns: Map<string, number>, columnName: string): string {
  const index = columns.get(columnName);
  if (index === undefined) {
    return "";
  }

  return (row[index] || "").trim();
}

function normalizeCatalogWebsite(website: string): string {
  try {
    const url = new URL(website.trim());
    url.hash = "";
    return url.toString();
  } catch {
    return website.trim();
  }
}

function compareCatalogDates(left: string, right: string): number {
  return left.localeCompare(right);
}

function combineCatalogTags(...groups: string[]): string[] {
  const seen = new Set<string>();

  for (const group of groups) {
    for (const tag of group.split(",")) {
      const cleaned = tag.trim();
      if (cleaned) {
        seen.add(cleaned);
      }
    }
  }

  return [...seen];
}

function buildCatalogKey(entry: CatalogEntry): string {
  return `${entry.website}::${entry.name}`;
}

function buildOptionList(values: Array<string | undefined>): string[] {
  return [...new Set(values.map((value) => value?.trim()).filter((value): value is string => Boolean(value)))].sort((left, right) => left.localeCompare(right));
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

function extractHostLabel(website: string): string | null {
  try {
    return new URL(website).hostname.replace(/^www\./, "");
  } catch {
    return null;
  }
}

function getTypeBadgeClass(type: string | undefined): string {
  const normalized = (type || "").toLowerCase();

  if (normalized === "nsfw") {
    return "badge-type-nsfw";
  }

  if (normalized === "sfw") {
    return "badge-type-sfw";
  }

  return "badge-type-default";
}
</script>

<template>
  <div class="catalog-view">
    <div class="toolbar">
      <input
        v-model="search"
        class="search"
        type="text"
        placeholder="Name, author, universe, tags, description…"
      />

      <select v-model="sort" class="filter-select" title="Sort">
        <option value="date">Newest first</option>
        <option value="name">Name A-Z</option>
      </select>

      <div>
        Catalog entries come from
        <b><a class="source-link" href="https://docs.google.com/spreadsheets/d/1jxBbWB08myhD8YXePPifsWQG3JH2qZtBs9Y5yYcqE7g" target="_blank" rel="noreferrer">Google Sheets</a></b>
        with <b><a class="source-link" href="https://infaera.neocities.org/ZipArchive/" target="_blank" rel="noreferrer">ZIP archive</a></b> fallback if available.
      </div>

      <div class="toolbar-spacer" />

      <button class="btn-secondary" :disabled="loading" @click="loadCatalog">
        {{ loading ? "Loading…" : "Reload Catalog" }}
      </button>
    </div>

    <div class="filter-row">
      <select v-model="authorFilter" class="filter-select">
        <option value="">All authors</option>
        <option v-for="option in authorOptions" :key="option" :value="option">{{ option }}</option>
      </select>

      <select v-model="universeFilter" class="filter-select">
        <option value="">All universes</option>
        <option v-for="option in universeOptions" :key="option" :value="option">{{ option }}</option>
      </select>

      <select v-model="importerFilter" class="filter-select">
        <option value="">All importers</option>
        <option v-for="option in importerOptions" :key="option" :value="option">{{ option }}</option>
      </select>

      <select v-model="typeFilter" class="filter-select">
        <option value="">All types</option>
        <option v-for="option in typeOptions" :key="option" :value="option">{{ option }}</option>
      </select>

      <select v-model="povFilter" class="filter-select">
        <option value="">All PoV</option>
        <option v-for="option in povOptions" :key="option" :value="option">{{ option }}</option>
      </select>

      <select v-model="lengthFilter" class="filter-select">
        <option value="">All lengths</option>
        <option v-for="option in lengthOptions" :key="option" :value="option">{{ option }}</option>
      </select>

      <select v-model="tagFilter" class="filter-select">
        <option value="">All tags</option>
        <option v-for="option in tagOptions" :key="option" :value="option">{{ option }}</option>
      </select>
    </div>

    <div class="status-bar">
      <span>Source: {{ sourceLabel }}</span>
      <span>{{ displayedList.length }} / {{ entries.length }} entries</span>
    </div>

    <div v-if="successMessage" class="banner success">{{ successMessage }}</div>
    <div v-if="error" class="banner error">{{ error }}</div>

    <div v-if="loading" class="center-msg">Loading catalog…</div>
    <div v-else-if="displayedList.length === 0" class="center-msg">No catalog entries match your filter.</div>

    <div v-else class="grid">
      <article v-for="entry in displayedList" :key="entry.catalogKey" class="card">
        <div v-if="entry.type" class="badge badge-type" :class="getTypeBadgeClass(entry.type)">{{ entry.type }}</div>
        <div class="info">
          <div class="card-head">
            <div class="title-block">
              <h3 class="name" :title="entry.name">{{ entry.name }}</h3>
              <div v-if="entry.author" class="author-line">
                <span class="author-label">Author</span>
                <span class="author-name">{{ entry.author }}</span>
              </div>
              <div class="byline">
                <span v-if="entry.universe">{{ entry.universe }}</span>
                <span v-if="entry.hostLabel">{{ entry.hostLabel }}</span>
                <span v-if="entry.importer">Imported by {{ entry.importer }}</span>
              </div>
            </div>
            <div class="badge badge-date">{{ entry.date }}</div>
          </div>

          <div class="meta-grid">
            <div v-if="entry.pov" class="meta-item">
              <span class="meta-label">PoV</span>
              <span>{{ entry.pov }}</span>
            </div>
            <div v-if="entry.length" class="meta-item">
              <span class="meta-label">Length</span>
              <span>{{ entry.length }}</span>
            </div>
            <div v-if="entry.universe" class="meta-item meta-item-highlight">
              <span class="meta-label">Universe</span>
              <span>{{ entry.universe }}</span>
            </div>
          </div>

          <p v-if="entry.description" class="description">{{ entry.description }}</p>

          <div v-if="entry.tags?.length" class="tag-list">
            <span v-for="tag in entry.tags" :key="tag" class="tag-chip">{{ tag }}</span>
          </div>

          <div v-if="addingLink === entry.catalogKey" class="progress-wrap">
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
              {{ addingLink === entry.catalogKey ? "Adding…" : "Add to Library" }}
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
  --catalog-card-top-tint: rgba(110, 160, 255, 0.08);
  --catalog-card-side-tint: rgba(255, 160, 120, 0.08);
  --catalog-card-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
  --catalog-date-bg: rgba(9, 16, 28, 0.58);
  --catalog-date-border: rgba(151, 180, 228, 0.16);
  --catalog-date-color: #f7f9ff;
  --catalog-author-label: rgba(255, 205, 145, 0.75);
  --catalog-author-name: #ffdca8;
  --catalog-byline: rgba(220, 229, 245, 0.72);
  --catalog-meta-bg: rgba(255, 255, 255, 0.045);
  --catalog-meta-border: rgba(165, 184, 214, 0.16);
  --catalog-meta-highlight-bg: rgba(255, 220, 140, 0.08);
  --catalog-meta-highlight-border: rgba(255, 210, 132, 0.26);
  --catalog-meta-label: rgba(145, 154, 176, 0.8);
  --catalog-description: rgba(225, 232, 244, 0.8);
  --catalog-tag-bg: rgba(107, 163, 255, 0.16);
  --catalog-tag-border: rgba(107, 163, 255, 0.28);
  --catalog-tag-color: #d4e4ff;
  --catalog-source-link: #9ec5ff;
}

:global(html:not(.dark) .catalog-view),
:global(:root:not(.dark) .catalog-view) {
  --catalog-card-top-tint: rgba(73, 119, 221, 0.13);
  --catalog-card-side-tint: rgba(255, 166, 94, 0.16);
  --catalog-card-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.84), 0 12px 28px rgba(42, 56, 92, 0.1);
  --catalog-date-bg: rgba(255, 255, 255, 0.92);
  --catalog-date-border: rgba(85, 111, 167, 0.26);
  --catalog-date-color: #243555;
  --catalog-author-label: #915018;
  --catalog-author-name: #6e2605;
  --catalog-byline: #42506a;
  --catalog-meta-bg: rgba(255, 255, 255, 0.9);
  --catalog-meta-border: rgba(96, 118, 161, 0.24);
  --catalog-meta-highlight-bg: rgba(255, 214, 122, 0.22);
  --catalog-meta-highlight-border: rgba(184, 129, 34, 0.36);
  --catalog-meta-label: #5b667b;
  --catalog-description: #334055;
  --catalog-tag-bg: rgba(58, 108, 232, 0.14);
  --catalog-tag-border: rgba(58, 108, 232, 0.26);
  --catalog-tag-color: #2348a5;
  --catalog-source-link: #2458c6;
}

.source-link {
  color: var(--catalog-source-link);
}

.source-link:hover {
  color: var(--accent-hover);
}

.toolbar {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
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
  max-width: 200px;
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

.filter-row {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  padding: 12px 20px 0;
}

.status-bar {
  display: flex;
  justify-content: space-between;
  flex-wrap: wrap;
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
  position: relative;
  background:
    linear-gradient(180deg, var(--catalog-card-top-tint), rgba(110, 160, 255, 0) 120px),
    linear-gradient(135deg, var(--catalog-card-side-tint), transparent 55%),
    var(--card-bg);
  border: 1px solid var(--border);
  box-shadow: var(--catalog-card-shadow);
  border-radius: 14px;
  padding: 16px;
  display: flex;
  flex-direction: column;
  overflow: hidden;
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

.badge-date {
  background: var(--catalog-date-bg);
  border-color: var(--catalog-date-border);
  color: var(--catalog-date-color);
}

.badge-type {
  position: absolute;
  top: 14px;
  right: 14px;
  z-index: 1;
  padding: 6px 11px;
  font-size: 0.7rem;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.badge-type-nsfw {
  background: linear-gradient(135deg, rgba(255, 100, 124, 0.95), rgba(255, 152, 84, 0.95));
  color: #fff6f1;
}

.badge-type-sfw {
  background: linear-gradient(135deg, rgba(70, 191, 166, 0.95), rgba(80, 134, 255, 0.95));
  color: #f3fffd;
}

.badge-type-default {
  background: linear-gradient(135deg, rgba(114, 123, 144, 0.95), rgba(83, 92, 110, 0.95));
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
  gap: 18px;
  flex-wrap: wrap;
  padding-right: 88px;
}

.title-block {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.name {
  margin: 0;
  font-size: 1.06rem;
  color: var(--text);
}

.author-line {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.author-label {
  color: var(--catalog-author-label);
  font-size: 0.72rem;
  letter-spacing: 0.1em;
  text-transform: uppercase;
}

.author-name {
  color: var(--catalog-author-name);
  font-size: 0.92rem;
  font-weight: 700;
}

.byline {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  color: var(--catalog-byline);
  font-size: 0.82rem;
}

.byline span::after {
  content: "•";
  margin-left: 8px;
  opacity: 0.5;
}

.byline span:last-child::after {
  content: "";
  margin: 0;
}

.meta-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
  gap: 10px;
}

.meta-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 10px;
  border-radius: 8px;
  background: var(--catalog-meta-bg);
  border: 1px solid var(--catalog-meta-border);
  font-size: 0.82rem;
  color: var(--text);
}

.meta-item-highlight {
  background: var(--catalog-meta-highlight-bg);
  border-color: var(--catalog-meta-highlight-border);
}

.meta-label {
  color: var(--catalog-meta-label);
  font-size: 0.72rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.description {
  margin: 0;
  color: var(--catalog-description);
  font-size: 0.88rem;
  line-height: 1.5;
}

.tag-list {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.tag-chip {
  padding: 4px 10px;
  border-radius: 999px;
  background: var(--catalog-tag-bg);
  border: 1px solid var(--catalog-tag-border);
  color: var(--catalog-tag-color);
  font-size: 0.76rem;
}

.actions {
  display: flex;
  flex-wrap: wrap;
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