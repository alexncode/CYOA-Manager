<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import PerkCard from "../components/PerkCard.vue";
import ProgressBar from "../components/ProgressBar.vue";
import { useLibrary } from "../composables/useLibrary";
import type { PerkIndexStatus, PerkSearchResult } from "../types";

const { projects, getPerkIndexStatus, startPerkIndexTask, searchPerks } = useLibrary();

const query = ref("");
const results = ref<PerkSearchResult[]>([]);
const status = ref<PerkIndexStatus | null>(null);
const loading = ref(false);
const indexing = ref(false);
const error = ref<string | null>(null);
const hasMore = ref(false);
const showImagePrompt = ref(false);
const pendingForceRebuild = ref(false);
const indexTaskId = ref("");
const indexProgress = ref(0);
const indexProgressLabel = ref("Preparing perk index...");
const indexProgressDetails = ref("");
const resultsWrapRef = useTemplateRef<HTMLElement>("resultsWrapRef");
const sentinelRef = useTemplateRef<HTMLElement>("sentinelRef");
const pageSize = 60;
let indexProgressUnlisten: UnlistenFn | null = null;
let infiniteScrollObserver: IntersectionObserver | null = null;

type PerkIndexProgressPayload = {
  taskId: string;
  phase: string;
  current: number;
  total: number;
  message: string;
  done: boolean;
  success: boolean;
  error?: string | null;
  status?: PerkIndexStatus | null;
};

const canSearch = computed(() => Boolean(status.value?.ready) && projects.value.length > 0);
const resultCountLabel = computed(() => {
  if (!status.value?.ready || indexing.value) {
    return "";
  }

  if (loading.value && results.value.length === 0) {
    return "Searching...";
  }

  if (results.value.length === 0) {
    return "0 results";
  }

  if (hasMore.value) {
    return `${results.value.length}+ results`;
  }

  return `${results.value.length} result${results.value.length === 1 ? "" : "s"}`;
});

onMounted(async () => {
  await initializeIndex();
});

onBeforeUnmount(async () => {
  if (indexProgressUnlisten) {
    await indexProgressUnlisten();
    indexProgressUnlisten = null;
  }

  infiniteScrollObserver?.disconnect();
  infiniteScrollObserver = null;
});

async function initializeIndex() {
  try {
    status.value = await getPerkIndexStatus();
    if (!status.value.ready || status.value.needsReindex) {
      openImagePrompt(false);
    } else {
      await runSearch(true);
    }
  } catch (cause: any) {
    error.value = String(cause);
  }
}

function openImagePrompt(forceRebuild: boolean) {
  pendingForceRebuild.value = forceRebuild;
  showImagePrompt.value = true;
}

async function startIndexTask(includeImages: boolean, forceRebuild: boolean) {
  showImagePrompt.value = false;
  indexing.value = true;
  error.value = null;
  indexProgress.value = 3;
  indexProgressLabel.value = forceRebuild ? "Rebuilding perk index..." : "Building perk index...";
  indexProgressDetails.value = includeImages
    ? "Images enabled. This can take longer and use more disk space."
    : "Indexing without image extraction.";

  try {
    if (indexProgressUnlisten) {
      await indexProgressUnlisten();
      indexProgressUnlisten = null;
    }

    indexProgressUnlisten = await listen<PerkIndexProgressPayload>("perk-index-progress", async (event) => {
      const payload = event.payload;
      if (!indexTaskId.value || payload.taskId !== indexTaskId.value) {
        return;
      }

      indexProgressLabel.value = payload.message || indexProgressLabel.value;
      if (payload.total > 0) {
        indexProgress.value = Math.max(5, Math.min(99, Math.round((payload.current / payload.total) * 100)));
        indexProgressDetails.value = `${payload.current} / ${payload.total} projects`;
      }

      if (!payload.done) {
        return;
      }

      indexing.value = false;
      indexTaskId.value = "";
      if (payload.success) {
        indexProgress.value = 100;
        status.value = payload.status ?? await getPerkIndexStatus();
        await runSearch(true);
      } else {
        error.value = payload.error || payload.message || "Perk index failed.";
      }

      if (indexProgressUnlisten) {
        await indexProgressUnlisten();
        indexProgressUnlisten = null;
      }
    });

    indexTaskId.value = await startPerkIndexTask(includeImages, forceRebuild);
  } catch (cause: any) {
    error.value = String(cause);
    indexTaskId.value = "";
    indexing.value = false;
    if (indexProgressUnlisten) {
      await indexProgressUnlisten();
      indexProgressUnlisten = null;
    }
  }
}

async function runSearch(reset: boolean) {
  if (!status.value?.ready && !status.value?.needsReindex) {
    return;
  }

  loading.value = true;
  error.value = null;
  try {
    const offset = reset ? 0 : results.value.length;
    const next = await searchPerks(query.value, pageSize, offset);
    results.value = reset ? next : [...results.value, ...next];
    hasMore.value = next.length === pageSize;
    await nextTick();
    setupInfiniteScroll();
  } catch (cause: any) {
    error.value = String(cause);
  } finally {
    loading.value = false;
  }
}

async function rebuild(includeImages: boolean) {
  await startIndexTask(includeImages, true);
}

function setupInfiniteScroll() {
  infiniteScrollObserver?.disconnect();
  infiniteScrollObserver = null;

  if (!resultsWrapRef.value || !sentinelRef.value) {
    return;
  }

  infiniteScrollObserver = new IntersectionObserver(
    (entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) {
        return;
      }

      if (loading.value || indexing.value || !hasMore.value) {
        return;
      }

      void runSearch(false);
    },
    {
      root: resultsWrapRef.value,
      rootMargin: "500px 0px",
    },
  );

  infiniteScrollObserver.observe(sentinelRef.value);
}

watch(
  () => [hasMore.value, results.value.length] as const,
  async () => {
    await nextTick();
    setupInfiniteScroll();
  },
);
</script>

<template>
  <div class="perk-view">
    <div class="toolbar">
      <input
        v-model="query"
        class="search"
        type="text"
        placeholder="Search perk title, description, addons, row"
        :disabled="indexing"
        @keydown.enter="runSearch(true)"
      />
      <button class="btn-primary" :disabled="indexing || !canSearch" @click="runSearch(true)">
        Search
      </button>
      <span v-if="resultCountLabel" class="result-count">{{ resultCountLabel }}</span>
      <button class="btn-secondary" :disabled="indexing || projects.length === 0" @click="openImagePrompt(false)">
        Sync Index
      </button>
      <button class="btn-secondary" :disabled="indexing || projects.length === 0" @click="rebuild(true)">
        Rebuild With Images
      </button>
      <button class="btn-secondary" :disabled="indexing || projects.length === 0" @click="rebuild(false)">
        Rebuild Without Images
      </button>
    </div>

    <div class="status-bar">
      <span v-if="status">
        Indexed {{ status.indexedProjects }}/{{ status.totalProjects }} projects, {{ status.perkCount }} perks
      </span>
      <span v-if="status">
        Images: {{ status.imagesEnabled ? "enabled" : "disabled" }}
      </span>
      <span v-if="status?.lastIndexedAt">Last indexed: {{ new Date(status.lastIndexedAt).toLocaleString() }}</span>
    </div>

    <div v-if="projects.length === 0" class="center-msg">
      Add projects to the library before building the perk index.
    </div>
    <div v-else-if="indexing" class="center-msg">
      <div class="indexing-panel">
        <ProgressBar :label="indexProgressLabel" :value="indexProgress" :details="indexProgressDetails" />
      </div>
    </div>
    <div v-else-if="error" class="center-msg error">
      {{ error }}
    </div>
    <div v-else-if="!status?.ready" class="center-msg">
      The perk index is not ready yet.
    </div>
    <div v-else-if="results.length === 0 && !loading" class="center-msg">
      No perks matched this search.
    </div>
    <div v-else ref="resultsWrapRef" class="results-wrap">
      <div class="results-grid">
        <PerkCard v-for="perk in results" :key="`${perk.projectId}-${perk.objectId}-${perk.rowId}`" :perk="perk" />
      </div>
      <div ref="sentinelRef" class="results-sentinel" aria-hidden="true">
        <span v-if="loading && results.length > 0" class="results-loading">Loading more perks...</span>
      </div>
    </div>

    <div v-if="showImagePrompt" class="overlay" @click.self="showImagePrompt = false">
      <div class="dialog">
        <h2>Build perk index</h2>
        <p>Include extracted images in the perk index?</p>
        <p class="warning-text">
          Image extraction can take longer and use a lot more disk space, especially on large libraries.
        </p>
        <div class="dialog-actions">
          <button class="btn-secondary" @click="showImagePrompt = false">Cancel</button>
          <button class="btn-secondary" @click="startIndexTask(false, pendingForceRebuild)">Without Images</button>
          <button class="btn-primary" @click="startIndexTask(true, pendingForceRebuild)">Include Images</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.perk-view {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 20px;
  border-bottom: 1px solid var(--border);
  background: var(--bg);
}

.search {
  flex: 1;
  min-width: 0;
  padding: 8px 12px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--text);
}

.result-count {
  color: var(--muted);
  font-size: 0.84rem;
  white-space: nowrap;
}

.status-bar {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
  padding: 10px 20px;
  color: var(--muted);
  font-size: 0.84rem;
  border-bottom: 1px solid var(--border);
}

.results-wrap {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 18px 20px 24px;
}

.results-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 16px;
}

.center-msg {
  flex: 1;
  display: grid;
  place-items: center;
  padding: 30px;
  text-align: center;
  color: var(--muted);
}

.error {
  color: #cf5a5a;
}

.results-sentinel {
  display: grid;
  place-items: center;
  min-height: 48px;
  margin-top: 12px;
}

.results-loading {
  color: var(--muted);
  font-size: 0.85rem;
}

.indexing-panel {
  width: min(520px, 100%);
  padding: 20px;
  background: var(--card-bg);
  border: 1px solid var(--border);
  border-radius: 14px;
}

.overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.46);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  z-index: 30;
}

.dialog {
  width: min(520px, 100%);
  background: var(--dialog-bg);
  border: 1px solid var(--border);
  border-radius: 14px;
  padding: 20px;
}

.dialog h2 {
  margin: 0 0 12px;
}

.dialog p {
  margin: 0 0 12px;
  line-height: 1.45;
}

.warning-text {
  color: var(--muted);
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 18px;
}

@media (max-width: 900px) {
  .toolbar {
    flex-wrap: wrap;
  }

  .search {
    width: 100%;
  }
}
</style>