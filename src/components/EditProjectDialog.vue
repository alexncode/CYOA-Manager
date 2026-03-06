<script setup lang="ts">
import { ref, watch } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { Project, ProjectPatch, Viewer } from "../types";

const props = defineProps<{
  project: Project;
  viewers: Viewer[];
}>();

const emit = defineEmits<{
  (e: "save", patch: ProjectPatch): void;
  (e: "close"): void;
}>();

const name = ref(props.project.name);
const description = ref(props.project.description);
const tagsRaw = ref(props.project.tags.join(", "));
const cover = ref(props.project.cover_image ?? "");
const viewerPreference = ref(props.project.viewer_preference ?? "");
const coverPreviewError = ref(false);

watch(() => cover.value, () => { coverPreviewError.value = false; });
watch(() => props.project, (project) => {
  name.value = project.name;
  description.value = project.description;
  tagsRaw.value = project.tags.join(", ");
  cover.value = project.cover_image ?? "";
  viewerPreference.value = project.viewer_preference ?? "";
}, { deep: true });

async function pickCover() {
  const selected = await open({
    title: "Select cover image",
    filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "gif"] }],
  });
  if (selected) cover.value = selected as string;
}

function save() {
  const patch: ProjectPatch = {
    name: name.value.trim() || props.project.name,
    description: description.value,
    cover_image: cover.value,
    viewer_preference: viewerPreference.value,
    tags: tagsRaw.value
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean),
  };
  emit("save", patch);
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog">
      <h2>Edit Project</h2>

      <label>Name
        <input v-model="name" type="text" placeholder="Project name" />
      </label>

      <label>Description
        <textarea v-model="description" rows="2" placeholder="Optional description" />
      </label>

      <label>Tags <span class="hint">(comma-separated)</span>
        <input v-model="tagsRaw" type="text" placeholder="tag1, tag2" />
      </label>

      <label>Preferred viewer
        <select v-model="viewerPreference">
          <option value="">No preference</option>
          <option v-for="viewer in viewers" :key="viewer.id" :value="viewer.id">
            {{ viewer.name }}
          </option>
        </select>
      </label>

      <label>Cover image URL or path
        <div class="cover-row">
          <input v-model="cover" type="text" placeholder="https://... or leave empty" />
          <button class="btn-secondary" @click="pickCover">Browse</button>
        </div>
        <img
          v-if="cover && !coverPreviewError"
          :src="cover"
          class="cover-preview"
          @error="coverPreviewError = true"
          alt="cover preview"
        />
      </label>

      <div class="file-path">
        <strong>File:</strong> {{ project.file_path }}
      </div>

      <div class="dialog-actions">
        <button class="btn-secondary" @click="emit('close')">Cancel</button>
        <button class="btn-primary" @click="save">Save</button>
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
  width: 480px;
  max-width: 95vw;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
h2 {
  margin: 0 0 4px;
  font-size: 1.2rem;
}
label {
  display: flex;
  flex-direction: column;
  gap: 5px;
  font-size: 0.875rem;
  color: var(--muted);
}
label input,
label textarea,
label select {
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text);
  padding: 7px 10px;
  font-size: 0.9rem;
  outline: none;
  transition: border-color 0.15s;
}
label input:focus,
label textarea:focus,
label select:focus {
  border-color: var(--accent);
}
.hint {
  font-size: 0.75rem;
  opacity: 0.6;
}
.cover-row {
  display: flex;
  gap: 8px;
}
.cover-row input {
  flex: 1;
}
.cover-preview {
  margin-top: 6px;
  height: 80px;
  width: 100%;
  object-fit: cover;
  border-radius: 6px;
}
.file-path {
  font-size: 0.75rem;
  color: var(--muted);
  word-break: break-all;
}
.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 4px;
}
</style>
