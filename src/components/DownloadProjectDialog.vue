<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useLibrary } from "../composables/useLibrary";
import { useSettings } from "../composables/useSettings";
import ProgressBar from "./ProgressBar.vue";
import OversizeActionPrompt from "./OversizeActionPrompt.vue";
import type { OversizeActionStrategy } from "../types";

const emit = defineEmits<{
  (e: "added"): void;
  (e: "close"): void;
}>();

const { startDownloadProject, startApplyOversizeProjectAction, loadViewers } = useLibrary();
const { settings } = useSettings();

const url = ref("");
const downloadIncludedIccPlusViewer = ref(false);
const downloading = ref(false);
const errorMsg = ref("");
const progress = ref(0);
const status = ref("");
const imageCurrent = ref(0);
const imageTotal = ref(0);
const downloadedSizeText = ref("");
let unlisten: UnlistenFn | null = null;
let oversizeUnlisten: UnlistenFn | null = null;
const showOversizePrompt = ref(false);
const oversizeProjectId = ref("");
const oversizeMb = ref(0);
const limitMb = ref(0);
const oversizeActionInProgress = ref(false);

const progressPercent = computed(() => Math.max(0, Math.min(100, Math.round(progress.value * 100))));
const progressLabel = computed(() => {
  const cleaned = stripMegabyteText(status.value).trim();
  return cleaned || "Downloading…";
});
const progressSubtext = computed(() => {
  const parts: string[] = [];
  if (downloadedSizeText.value) {
    parts.push(`Downloaded: ${downloadedSizeText.value}`);
  }
  if (imageTotal.value > 0) {
    parts.push(`Images: ${imageCurrent.value} / ${imageTotal.value}`);
  }
  return parts.join(" | ");
});

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

type OversizeActionPayload = {
  taskId: string;
  projectId: string;
  phase: string;
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
  if (oversizeUnlisten) {
    void oversizeUnlisten();
    oversizeUnlisten = null;
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
  downloadedSizeText.value = "";
  showOversizePrompt.value = false;
  oversizeProjectId.value = "";
  oversizeActionInProgress.value = false;

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
      downloadedSizeText.value = extractMegabyteText(payload.message);
      progress.value = payload.total > 0 ? payload.current / payload.total : 0;

      if (payload.done) {
        downloading.value = false;
        if (unlisten) {
          await unlisten();
          unlisten = null;
        }

        if (payload.success) {
          if (downloadIncludedIccPlusViewer.value) {
            await loadViewers();
          }
          emit("added");
          emit("close");
        } else {
          const err = payload.error || "Download failed.";
          if (parseOversizeError(err)) {
            downloading.value = false;
            const autoAction = getAutoOversizeAction();
            if (autoAction) {
              void chooseOversizeOption(autoAction);
            } else {
              showOversizePrompt.value = true;
            }
          } else {
            errorMsg.value = err;
          }
        }
      }
    });
    taskId = await startDownloadProject(
      trimmed,
      Math.max(1, Math.floor(settings.value.downloadSizeLimitMb || 200)),
      downloadIncludedIccPlusViewer.value,
    );
  } catch (e: any) {
    downloading.value = false;
    errorMsg.value = String(e);
  }
}

function extractMegabyteText(message: string): string {
  const match = message.match(/\b\d+(?:\.\d+)?\s*MB\b/i);
  return match ? match[0].toUpperCase() : "";
}

function stripMegabyteText(message: string): string {
  return message.replace(/\s*\(?\b\d+(?:\.\d+)?\s*MB\b\)?/gi, "").replace(/\s{2,}/g, " ");
}

function parseOversizeError(error: string): boolean {
  // Format: OVERSIZE|<project_id>|<size_bytes>|<limit_bytes>
  if (!error.startsWith("OVERSIZE|")) {
    return false;
  }

  const parts = error.split("|");
  if (parts.length < 4) {
    return false;
  }

  const projectId = parts[1] || "";
  const sizeBytes = Number(parts[2]);
  const maxBytes = Number(parts[3]);
  if (!Number.isFinite(sizeBytes) || !Number.isFinite(maxBytes) || maxBytes <= 0) {
    return false;
  }

  if (!projectId) {
    return false;
  }

  oversizeProjectId.value = projectId;
  oversizeMb.value = Math.round((sizeBytes / (1024 * 1024)) * 10) / 10;
  limitMb.value = Math.round((maxBytes / (1024 * 1024)) * 10) / 10;
  return true;
}

async function chooseOversizeOption(strategy: OversizeActionStrategy): Promise<boolean> {
  if (!oversizeProjectId.value || oversizeActionInProgress.value) {
    return false;
  }

  oversizeActionInProgress.value = true;
  downloading.value = true;
  errorMsg.value = "";
  status.value = "Applying post-download action…";

  return new Promise<boolean>(async (resolve) => {
    try {
      if (oversizeUnlisten) {
        await oversizeUnlisten();
        oversizeUnlisten = null;
      }

      let taskId = "";
      let pendingPayload: OversizeActionPayload | null = null;

      let handlePayload: ((payload: OversizeActionPayload) => Promise<void>) | null = null;
      handlePayload = async (payload: OversizeActionPayload) => {
        if (!taskId) {
          pendingPayload = payload;
          return;
        }

        if (payload.taskId !== taskId) {
          return;
        }

        status.value = payload.message;
        if (!payload.done) {
          return;
        }

        oversizeActionInProgress.value = false;
        downloading.value = false;
        if (oversizeUnlisten) {
          await oversizeUnlisten();
          oversizeUnlisten = null;
        }

        if (payload.success) {
          showOversizePrompt.value = false;
          emit("added");
          emit("close");
          resolve(true);
        } else {
          errorMsg.value = payload.error || "Oversize action failed.";
          resolve(false);
        }
      };

      oversizeUnlisten = await listen<OversizeActionPayload>("oversize-action-progress", async (event) => {
        const payload = event.payload;
        if (handlePayload) {
          await handlePayload(payload);
        }
      });

      taskId = await startApplyOversizeProjectAction(oversizeProjectId.value, strategy);
      if (pendingPayload && handlePayload) {
        const bufferedPayload = pendingPayload;
        pendingPayload = null;
        await handlePayload(bufferedPayload);
      }
    } catch (error) {
      oversizeActionInProgress.value = false;
      downloading.value = false;
      errorMsg.value = String(error);
      resolve(false);
    }
  });
}

function getAutoOversizeAction(): OversizeActionStrategy | null {
  const action = settings.value.oversizeDefaultAction;
  return action === "ask" ? null : action;
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

      <label class="viewer-toggle">
        <span class="viewer-toggle-row">
          <input
            v-model="downloadIncludedIccPlusViewer"
            type="checkbox"
            :disabled="downloading"
          />
          <span>Download with a included ICC+ viewer</span>
        </span>
      </label>

      <p class="viewer-hint">
        Checks the site for an ICC+ style viewer bundle and saves it into the local viewers folder with the project version when possible.
      </p>

      <ProgressBar
        v-if="downloading"
        :label="progressLabel"
        :value="progressPercent"
        :details="progressSubtext"
      />

      <div v-if="errorMsg" class="error">{{ errorMsg }}</div>

      <div v-if="showOversizePrompt" class="oversize-prompt">
        <OversizeActionPrompt
          :final-size-mb="oversizeMb"
          :limit-mb="limitMb"
          :busy="oversizeActionInProgress"
          :status="status"
          @choose="chooseOversizeOption"
        />
      </div>

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
.viewer-toggle {
  gap: 0;
}
.viewer-toggle-row {
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--text);
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
.viewer-toggle input {
  width: 16px;
  height: 16px;
  padding: 0;
}
.viewer-hint {
  margin: -8px 0 0;
  font-size: 0.85rem;
  line-height: 1.4;
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
.oversize-prompt {
  margin-top: 4px;
  padding: 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--input-bg);
}
</style>