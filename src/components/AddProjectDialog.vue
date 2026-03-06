<script setup lang="ts">
import { ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { useLibrary } from "../composables/useLibrary";

const emit = defineEmits<{
  (e: "added"): void;
  (e: "close"): void;
}>();

const { addProject } = useLibrary();

const importing = ref(false);
const errorMsg = ref("");

async function pickAndAdd() {
  const selected = await open({
    title: "Select project.json",
    filters: [{ name: "CYOA Project", extensions: ["json"] }],
    multiple: false,
  });
  if (!selected) return;
  await doAdd(selected as string);
}

async function doAdd(path: string) {
  importing.value = true;
  errorMsg.value = "";
  try {
    await addProject(path);
    emit("added");
    emit("close");
  } catch (e: any) {
    errorMsg.value = String(e);
  } finally {
    importing.value = false;
  }
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog">
      <h2>Add Project</h2>
      <p class="sub">Select a <code>project.json</code> file to add to your library.</p>

      <div v-if="errorMsg" class="error">{{ errorMsg }}</div>

      <div class="dialog-actions">
        <button class="btn-secondary" @click="emit('close')">Cancel</button>
        <button class="btn-primary" :disabled="importing" @click="pickAndAdd">
          {{ importing ? "Adding…" : "Browse file" }}
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
  width: 400px;
  max-width: 95vw;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
h2 { margin: 0; }
.sub { margin: 0; font-size: 0.9rem; color: var(--muted); }
code { color: var(--accent); }
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
