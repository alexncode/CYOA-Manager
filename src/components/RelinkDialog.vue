<script setup lang="ts">
import { ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { Project, ProjectPatch } from "../types";

const props = defineProps<{ project: Project }>();
const emit = defineEmits<{
  (e: "save", patch: ProjectPatch): void;
  (e: "close"): void;
}>();

const newPath = ref(props.project.file_path);

async function browse() {
  const selected = await open({
    title: "Select project.json",
    filters: [{ name: "CYOA Project", extensions: ["json"] }],
  });
  if (selected) newPath.value = selected as string;
}

function save() {
  emit("save", { file_path: newPath.value });
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog">
      <h2>Re-link File</h2>
      <p class="sub">
        The file for <strong>{{ project.name }}</strong> is missing. Select the new location.
      </p>
      <div class="row">
        <input v-model="newPath" type="text" placeholder="Path to project.json" />
        <button class="btn-secondary" @click="browse">Browse</button>
      </div>
      <div class="dialog-actions">
        <button class="btn-secondary" @click="emit('close')">Cancel</button>
        <button class="btn-primary" :disabled="!newPath" @click="save">Save</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed; inset: 0; background: rgba(0,0,0,.6);
  display: flex; align-items: center; justify-content: center; z-index: 100;
}
.dialog {
  background: var(--dialog-bg); border: 1px solid var(--border); border-radius: 12px;
  padding: 28px; width: 480px; max-width: 95vw;
  display: flex; flex-direction: column; gap: 14px;
}
h2 { margin: 0; }
.sub { margin: 0; font-size: .9rem; color: var(--muted); }
.row { display: flex; gap: 8px; }
.row input {
  flex: 1; background: var(--input-bg); border: 1px solid var(--border);
  border-radius: 6px; color: var(--text); padding: 7px 10px; font-size: .9rem; outline: none;
}
.row input:focus { border-color: var(--accent); }
.dialog-actions { display: flex; justify-content: flex-end; gap: 10px; }
</style>
