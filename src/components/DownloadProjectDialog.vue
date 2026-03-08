<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useLibrary } from "../composables/useLibrary";

const emit = defineEmits<{
  (e: "added"): void;
  (e: "close"): void;
}>();

const { startDownloadProject } = useLibrary();

const url = ref("");
const downloading = ref(false);
const errorMsg = ref("");
const progress = ref(0);
const status = ref("");
const imageCurrent = ref(0);
const imageTotal = ref(0);
let unlisten: UnlistenFn | null = null;

const progressPercent = computed(() => Math.max(0, Math.min(100, Math.round(progress.value * 100))));

type ProgressPayload = {
  taskId: string;
  phase: string;
  current: number;
  total: number;
  imageCurrent: number;
  imageTotal: number;
  message: string;
  done: boolean;
  success: boolean;
  error?: string | null;
};

onBeforeUnmount(() => {
  if (unlisten) {
    void unlisten();
    unlisten = null;
  }
});

async function submit() {
  const trimmed = url.value.trim();
  if (!trimmed) {
    errorMsg.value = "Enter a project URL.";
    return;
  }

  downloading.value = true;
  errorMsg.value = "";
  status.value = "Preparing download…";
  progress.value = 0;
  imageCurrent.value = 0;
  imageTotal.value = 0;

  try {
    if (unlisten) {
      await unlisten();
      unlisten = null;
    }

    let taskId = "";
    unlisten = await listen<ProgressPayload>("download-project-progress", async (event) => {
      const payload = event.payload;
      if (!taskId || payload.taskId !== taskId) return;

      status.value = payload.message;
      imageCurrent.value = payload.imageCurrent;
      imageTotal.value = payload.imageTotal;
      progress.value = payload.total > 0 ? payload.current / payload.total : 0;

      if (payload.done) {
        downloading.value = false;
        if (unlisten) {
          await unlisten();
          unlisten = null;
        }

        if (payload.success) {
          emit("added");
          emit("close");
        } else {
          errorMsg.value = payload.error || "Download failed.";
        }
      }
    });

    taskId = await startDownloadProject(trimmed);
  } catch (e: any) {
    downloading.value = false;
    errorMsg.value = String(e);
  }
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog">
      <h2>Download Project</h2>

      <p class="sub">
        Paste a project URL, a direct JSON link, or a cyoa.cafe page URL. Linked images will be downloaded in the background and inlined automatically.
      </p>

      <label>
        CYOA URL
        <input
          v-model="url"
          type="url"
          placeholder="Example: https://om1cr0n.nekoweb.org/albedo/ or https://example.com/albedo-data.json"
          @keydown.enter.prevent="submit"
          :disabled="downloading"
        />
      </label>

      <div v-if="downloading" class="progress-wrap">
        <div class="progress-meta">
          <span>{{ status || "Downloading…" }}</span>
          <span>{{ progressPercent }}%</span>
        </div>
        <div class="progress-bar">
          <div class="progress-fill" :style="{ width: `${progressPercent}%` }" />
        </div>
        <div class="progress-sub" v-if="imageTotal > 0">
          Images: {{ imageCurrent }} / {{ imageTotal }}
        </div>
      </div>

      <div v-if="errorMsg" class="error">{{ errorMsg }}</div>

      <div class="dialog-actions">
        <button class="btn-secondary" :disabled="downloading" @click="emit('close')">Cancel</button>
        <button class="btn-primary" :disabled="downloading" @click="submit">
          {{ downloading ? "Downloading…" : "Download" }}
        </button>
      </div>
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
  width: 460px;
  max-width: 95vw;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
h2 { margin: 0; }
.sub { margin: 0; font-size: 0.9rem; color: var(--muted); }
label {
  display: flex;
  flex-direction: column;
  gap: 6px;
  color: var(--muted);
  font-size: 0.9rem;
}
input {
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text);
  padding: 9px 10px;
  font-size: 0.95rem;
  outline: none;
}
input:focus {
  border-color: var(--accent);
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
  font-size: 0.85rem;
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
.progress-sub {
  font-size: 0.82rem;
  color: var(--muted);
}
.error {
  background: rgba(200, 50, 50, 0.15);
  border: 1px solid #c33;
  border-radius: 6px;
  padding: 8px 12px;
  font-size: 0.85rem;
  color: #e88;
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}
</style>