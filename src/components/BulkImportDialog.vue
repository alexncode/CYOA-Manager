<script setup lang="ts">
import { ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { useLibrary } from "../composables/useLibrary";

const emit = defineEmits<{
  (e: "added"): void;
  (e: "close"): void;
}>();

const { scanFolder } = useLibrary();

const discovered = ref<string[]>([]);
const selected = ref<Set<string>>(new Set());
const scanning = ref(false);
const importing = ref(false);
const progress = ref(0);
const step = ref<"pick" | "select" | "done">("pick");

async function pickFolder() {
  const folder = await open({ directory: true, title: "Select folder to scan" });
  if (!folder) return;
  scanning.value = true;
  try {
    discovered.value = await scanFolder(folder as string);
    selected.value = new Set(discovered.value);
    step.value = "select";
  } catch (e) {
    console.error(e);
  } finally {
    scanning.value = false;
  }
}

function toggle(path: string) {
  if (selected.value.has(path)) selected.value.delete(path);
  else selected.value.add(path);
  // force reactivity
  selected.value = new Set(selected.value);
}

function selectAll() {
  selected.value = new Set(discovered.value);
}

function selectNone() {
  selected.value = new Set();
}

async function importSelected() {
  const paths = discovered.value.filter((p) => selected.value.has(p));
  if (!paths.length) return;
  importing.value = true;
  progress.value = 0;
  for (let i = 0; i < paths.length; i++) {
    try {
      const { addProject } = useLibrary();
      await addProject(paths[i]);
    } catch (e) {
      console.error("skip:", paths[i], e);
    }
    progress.value = i + 1;
  }
  importing.value = false;
  step.value = "done";
  emit("added");
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog">
      <h2>Bulk Import</h2>

      <!-- Step: pick folder -->
      <template v-if="step === 'pick'">
        <p class="sub">Scan a folder for all <code>project.json</code> files.</p>
        <div class="dialog-actions">
          <button class="btn-secondary" @click="emit('close')">Cancel</button>
          <button class="btn-primary" :disabled="scanning" @click="pickFolder">
            {{ scanning ? "Scanning…" : "Select folder" }}
          </button>
        </div>
      </template>

      <!-- Step: review list -->
      <template v-else-if="step === 'select'">
        <div class="list-header">
          <span>{{ discovered.length }} projects found — {{ selected.size }} selected</span>
          <div class="select-actions">
            <button class="btn-ghost" @click="selectAll">All</button>
            <button class="btn-ghost" @click="selectNone">None</button>
          </div>
        </div>

        <div class="file-list">
          <label
            v-for="path in discovered"
            :key="path"
            class="file-item"
          >
            <input
              type="checkbox"
              :checked="selected.has(path)"
              @change="toggle(path)"
            />
            <span class="file-path">{{ path }}</span>
          </label>
          <div v-if="!discovered.length" class="empty">No project.json files found.</div>
        </div>

        <div v-if="importing" class="progress-bar">
          <div
            class="progress-fill"
            :style="{ width: (progress / selected.size * 100) + '%' }"
          />
          <span>{{ progress }} / {{ selected.size }}</span>
        </div>

        <div class="dialog-actions">
          <button class="btn-secondary" @click="emit('close')">Cancel</button>
          <button
            class="btn-primary"
            :disabled="importing || selected.size === 0"
            @click="importSelected"
          >
            {{ importing ? `Importing ${progress}/${selected.size}…` : `Import ${selected.size}` }}
          </button>
        </div>
      </template>

      <!-- Step: done -->
      <template v-else>
        <p class="sub">✅ Import complete!</p>
        <div class="dialog-actions">
          <button class="btn-primary" @click="emit('close')">Done</button>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}
.dialog {
  background: var(--dialog-bg);
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 28px;
  width: 540px;
  max-width: 95vw;
  max-height: 80vh;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
h2 { margin: 0; }
.sub { margin: 0; font-size: 0.9rem; color: var(--muted); }
code { color: var(--accent); }
.list-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 0.85rem;
  color: var(--muted);
}
.select-actions { display: flex; gap: 8px; }
.file-list {
  overflow-y: auto;
  flex: 1;
  min-height: 120px;
  max-height: 320px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 8px;
}
.file-item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  cursor: pointer;
  padding: 2px 0;
}
.file-item:hover { color: var(--accent); }
.file-path {
  font-size: 0.8rem;
  word-break: break-all;
}
.empty { color: var(--muted); font-size: 0.9rem; padding: 8px; }
.progress-bar {
  height: 20px;
  background: var(--border);
  border-radius: 10px;
  overflow: hidden;
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
}
.progress-fill {
  position: absolute;
  left: 0;
  top: 0;
  height: 100%;
  background: var(--accent);
  transition: width 0.2s;
}
.progress-bar span {
  position: relative;
  font-size: 0.75rem;
  z-index: 1;
}
.dialog-actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 4px; }
</style>
