<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Project, Viewer } from "../types";
import { resolveViewerId } from "../viewers";

const props = defineProps<{
  project: Project;
  viewers: Viewer[];
  defaultViewer: string | null;
  redownloadBusy?: boolean;
  redownloadLabel?: string | null;
}>();

const emit = defineEmits<{
  (e: "open", viewerId: string): void;
  (e: "remove"): void;
  (e: "edit"): void;
  (e: "relink"): void;
  (e: "redownload"): Promise<void> | void;
}>();

const menuOpen = ref(false);
const imageFailed = ref(false);
const coverImageSrc = ref<string | null>(null);
const openingSource = ref(false);
const redownloading = ref(false);
const selectedViewerId = ref<string | null>(null);

const initials = computed(() => {
  const words = props.project.name.trim().split(/\s+/);
  if (words.length >= 2) return (words[0][0] + words[1][0]).toUpperCase();
  return props.project.name.slice(0, 2).toUpperCase();
});

const coverColor = computed(() => {
  // deterministic pastel color from project id
  let hash = 0;
  for (const ch of props.project.id) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  const hue = hash % 360;
  return `hsl(${hue}, 40%, 35%)`;
});

const selectedViewer = computed(() => {
  return props.viewers.find((viewer) => viewer.id === selectedViewerId.value) ?? null;
});

const sourceUrl = computed(() => {
  const raw = props.project.source_url;
  return raw && raw.trim() ? raw : null;
});

const isRedownloading = computed(() => redownloading.value || Boolean(props.redownloadBusy));
const redownloadText = computed(() => {
  if (!isRedownloading.value) {
    return "Re-download";
  }

  return props.redownloadLabel?.trim() || "Re-downloading...";
});

watch(
  () => [props.project.file_path, props.project.cover_image],
  async () => {
    imageFailed.value = false;

    try {
      coverImageSrc.value = await invoke<string | null>("resolve_cover_image_src", {
        filePath: props.project.file_path,
        coverImage: props.project.cover_image,
      });
    } catch {
      coverImageSrc.value = null;
    }
  },
  { immediate: true }
);

watch(
  () => [
    props.project.id,
    props.project.viewer_preference,
    props.defaultViewer,
    props.viewers.map((viewer) => viewer.id).join("|"),
  ],
  () => {
    selectedViewerId.value = resolveViewerId(
      props.viewers,
      props.project.viewer_preference,
      props.defaultViewer,
    );
  },
  { immediate: true }
);

function openMenu() {
  menuOpen.value = !menuOpen.value;
}

function closeMenu() {
  menuOpen.value = false;
}

function onOpen(viewerId: string) {
  closeMenu();
  emit("open", viewerId);
}

function onImageError() {
  imageFailed.value = true;
}

async function onOpenSource() {
  if (!sourceUrl.value || openingSource.value) {
    return;
  }

  openingSource.value = true;
  try {
    await openUrl(sourceUrl.value);
  } catch (error) {
    console.error("Failed to open source URL:", error);
  } finally {
    openingSource.value = false;
  }
}

async function onRedownload() {
  if (!sourceUrl.value || isRedownloading.value) {
    return;
  }

  redownloading.value = true;
  try {
    await emit("redownload");
  } finally {
    redownloading.value = false;
  }
}
</script>

<template>
  <div
    class="card"
    :class="{ missing: project.file_missing }"
    @click.self="closeMenu"
  >
    <!-- Cover -->
    <div class="cover" :style="!coverImageSrc || imageFailed ? { background: coverColor } : {}">
      <img
        v-if="coverImageSrc && !imageFailed"
        :src="coverImageSrc"
        :alt="project.name"
        loading="lazy"
        @error="onImageError"
      />
      <span v-else class="initials">{{ initials }}</span>

      <div v-if="sourceUrl" class="source-actions">
        <button
          class="source-btn"
          :disabled="openingSource || isRedownloading"
          @click.stop="onOpenSource"
        >
          Open Source
        </button>
        <button
          class="source-btn secondary"
          :class="{ busy: isRedownloading }"
          :disabled="isRedownloading || openingSource"
          @click.stop="onRedownload"
        >
          {{ redownloadText }}
        </button>
      </div>

      <div v-if="project.file_missing" class="badge missing-badge">File missing</div>

      <!-- Menu button -->
      <button class="menu-btn" @click.stop="openMenu" title="Options">⋮</button>

      <!-- Overflow menu -->
      <div v-if="menuOpen" class="menu" @click.stop>
        <button @click="emit('edit'); closeMenu()">✏️ Edit</button>
        <button v-if="project.file_missing" @click="emit('relink'); closeMenu()">
          🔗 Re-link file
        </button>
        <button class="danger" @click="emit('remove'); closeMenu()">🗑 Remove</button>
      </div>
    </div>

    <!-- Info -->
    <div class="info">
      <h3 class="name" :title="project.name">{{ project.name }}</h3>

      <div v-if="project.tags.length" class="tags">
        <span v-for="tag in project.tags" :key="tag" class="tag">{{ tag }}</span>
      </div>

      <!-- Open action -->
      <div class="actions">
        <template v-if="viewers.length === 0">
          <span class="no-viewers">No viewers found</span>
        </template>
        <template v-else>
          <select v-model="selectedViewerId" class="viewer-select">
            <option v-for="v in viewers" :key="v.id" :value="v.id">{{ v.name }}</option>
          </select>
          <button
            class="btn-open"
            :disabled="project.file_missing || !selectedViewer"
            @click="selectedViewer && onOpen(selectedViewer.id)"
          >
            Open
          </button>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.card {
  background: var(--card-bg);
  border: 1px solid var(--border);
  border-radius: 10px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  transition: transform 0.15s, box-shadow 0.15s;
  position: relative;
}
.card:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
}
.card.missing {
  opacity: 0.7;
}

.cover {
  position: relative;
  height: 160px;
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--cover-placeholder);
}
.cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.initials {
  font-size: 2.5rem;
  font-weight: 700;
  color: rgba(255, 255, 255, 0.8);
  user-select: none;
}

.badge {
  position: absolute;
  top: 8px;
  left: 8px;
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 0.7rem;
  font-weight: 600;
}
.missing-badge {
  top: 42px;
  background: #e55;
  color: #fff;
}

.source-actions {
  position: absolute;
  top: 6px;
  left: 6px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.source-btn {
  padding: 6px 10px;
  background: rgba(0, 0, 0, 0.68);
  border: none;
  border-radius: 6px;
  color: #fff;
  font-size: 0.76rem;
  font-weight: 600;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.15s, background 0.15s;
}

.card:hover .source-btn {
  opacity: 1;
}

.source-btn.secondary {
  background: rgba(0, 0, 0, 0.56);
}

.source-btn.busy {
  background: rgba(18, 122, 96, 0.78);
}

.source-btn:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.82);
}

.source-btn:disabled {
  cursor: wait;
}

.menu-btn {
  position: absolute;
  top: 6px;
  right: 6px;
  background: rgba(0, 0, 0, 0.5);
  border: none;
  color: #fff;
  border-radius: 4px;
  font-size: 1.2rem;
  line-height: 1;
  padding: 2px 6px;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.15s;
}
.card:hover .menu-btn {
  opacity: 1;
}

.menu {
  position: absolute;
  top: 34px;
  right: 6px;
  background: var(--menu-bg);
  border: 1px solid var(--border);
  border-radius: 8px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
  z-index: 10;
  min-width: 150px;
  overflow: hidden;
}
.menu button {
  display: block;
  width: 100%;
  padding: 8px 14px;
  background: none;
  border: none;
  color: var(--text);
  text-align: left;
  cursor: pointer;
  font-size: 0.875rem;
}
.menu button:hover {
  background: var(--hover);
}
.menu button.danger {
  color: #e55;
}

.info {
  padding: 10px 12px 12px;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.name {
  margin: 0;
  font-size: 0.95rem;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.tag {
  background: var(--tag-bg);
  color: var(--tag-color);
  border-radius: 4px;
  padding: 1px 7px;
  font-size: 0.72rem;
}
.actions {
  margin-top: auto;
  display: flex;
  align-items: center;
  gap: 6px;
}
.viewer-select {
  flex: 1 1 auto;
  min-width: 0;
  padding: 5px 10px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text);
  font-size: 0.8rem;
  outline: none;
}
.viewer-select:focus {
  border-color: var(--accent);
}
.btn-open {
  flex: 0 0 auto;
  min-width: 72px;
  padding: 5px 10px;
  background: var(--accent);
  color: #fff;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  font-size: 0.8rem;
  font-weight: 600;
  transition: background 0.15s;
}
.btn-open:hover:not(:disabled) {
  background: var(--accent-hover);
}
.btn-open:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.no-viewers {
  font-size: 0.75rem;
  color: var(--muted);
}
</style>
