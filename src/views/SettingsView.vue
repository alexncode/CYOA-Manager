<script setup lang="ts">
import { ref, watch } from "vue";
import { useSettings } from "../composables/useSettings";
import { useLibrary } from "../composables/useLibrary";

const { settings, applyTheme } = useSettings();
const { viewers, clearLibrary } = useLibrary();
const clearingLibrary = ref(false);
const libraryActionError = ref<string | null>(null);
const libraryActionMessage = ref<string | null>(null);
const showClearLibraryConfirm = ref(false);

watch(() => settings.value.theme, applyTheme);

function onClearLibrary() {
  libraryActionError.value = null;
  libraryActionMessage.value = null;
  showClearLibraryConfirm.value = true;
}

function cancelClearLibrary() {
  showClearLibraryConfirm.value = false;
}

async function confirmClearLibrary() {
  try {
    clearingLibrary.value = true;
    showClearLibraryConfirm.value = false;
    libraryActionError.value = null;
    libraryActionMessage.value = null;
    await clearLibrary();
    libraryActionMessage.value = "Library cleared.";
  } catch (error) {
    libraryActionError.value = String(error);
  } finally {
    clearingLibrary.value = false;
  }
}
</script>

<template>
  <div class="settings-view">
    <h1>Settings</h1>

    <section class="section">
      <h2>Appearance</h2>
      <label class="row">
        <span>Theme</span>
        <select v-model="settings.theme">
          <option value="system">System default</option>
          <option value="dark">Dark</option>
          <option value="light">Light</option>
        </select>
      </label>
    </section>

    <section class="section">
      <h2>Viewers</h2>
      <label class="row">
        <span>Default viewer</span>
        <select v-model="settings.defaultViewer">
          <option :value="null">None (show all buttons)</option>
          <option v-for="v in viewers" :key="v.id" :value="v.id">{{ v.name }}</option>
        </select>
      </label>
      <p class="hint">
        When set, only one "Open" button appears on each card using this viewer.
      </p>
      <label class="row checkbox-row">
        <span>Cheats</span>
        <input v-model="settings.cheatsEnabled" type="checkbox" />
      </label>
      <p class="hint">
        Toggle the in-viewer cheat overlay menu.
      </p>
      <label class="row">
        <span>Download size limit (MB)</span>
        <input
          v-model.number="settings.downloadSizeLimitMb"
          type="number"
          min="50"
          max="2000"
          step="10"
        />
      </label>
      <p class="hint">
        If a downloaded project exceeds this size, you will be prompted for handling options.
      </p>
    </section>

    <section class="section">
      <h2>Library file location</h2>
      <p class="hint">
        The library index (<code>library.json</code>) is stored in a
        <code>save/</code> folder next to the application executable.
      </p>
    </section>

    <section class="section danger-section">
      <h2>Library Maintenance</h2>
      <p class="hint">
        Clear the saved library index without deleting the underlying project files.
      </p>
      <button
        class="btn-danger"
        :disabled="clearingLibrary"
        @click="onClearLibrary"
      >
        {{ clearingLibrary ? "Clearing..." : "Clear library" }}
      </button>
      <p v-if="libraryActionMessage" class="success-text">{{ libraryActionMessage }}</p>
      <p v-if="libraryActionError" class="error-text">{{ libraryActionError }}</p>
    </section>

    <div v-if="showClearLibraryConfirm" class="confirm-overlay" @click.self="cancelClearLibrary">
      <div class="confirm-dialog">
        <h3>Clear library?</h3>
        <p>
          This removes all cards from the saved library index. Project files on disk will not be deleted.
        </p>
        <div class="confirm-actions">
          <button class="btn-secondary" @click="cancelClearLibrary">Cancel</button>
          <button class="btn-danger" :disabled="clearingLibrary" @click="confirmClearLibrary">
            Delete all library entries
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.settings-view {
  padding: 32px 40px;
  max-width: 600px;
}
h1 {
  margin: 0 0 28px;
  font-size: 1.5rem;
}
.section {
  margin-bottom: 32px;
}
.section h2 {
  font-size: 1rem;
  font-weight: 600;
  color: var(--muted);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  margin: 0 0 14px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--border);
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  font-size: 0.9rem;
}
.checkbox-row {
  margin-top: 12px;
}
.checkbox-row input[type="checkbox"] {
  width: 16px;
  height: 16px;
  accent-color: var(--accent);
  cursor: pointer;
}
.row select {
  padding: 7px 10px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--text);
  font-size: 0.875rem;
  outline: none;
  min-width: 180px;
  cursor: pointer;
}
.row input[type="number"] {
  padding: 7px 10px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--text);
  font-size: 0.875rem;
  outline: none;
  min-width: 120px;
}
.row input[type="number"]:focus { border-color: var(--accent); }
.row select:focus { border-color: var(--accent); }
.hint {
  margin: 8px 0 0;
  font-size: 0.8rem;
  color: var(--muted);
}
.hint code { color: var(--accent); }
.danger-section {
  padding-top: 4px;
}
.btn-danger {
  background: #c53a3a;
  color: #fff;
  border: none;
  border-radius: 8px;
  padding: 8px 14px;
  font-size: 0.875rem;
  font-weight: 600;
  cursor: pointer;
}
.btn-secondary {
  background: var(--input-bg);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 8px 14px;
  font-size: 0.875rem;
  font-weight: 600;
  cursor: pointer;
}
.btn-danger:hover:not(:disabled) {
  background: #aa2f2f;
}
.btn-danger:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}
.error-text {
  margin-top: 10px;
  font-size: 0.8rem;
  color: #e55;
}
.success-text {
  margin-top: 10px;
  font-size: 0.8rem;
  color: #5fb36b;
}
.confirm-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 200;
}
.confirm-dialog {
  width: 420px;
  max-width: calc(100vw - 32px);
  background: var(--dialog-bg);
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 20px;
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.35);
}
.confirm-dialog h3 {
  margin: 0 0 10px;
  font-size: 1rem;
}
.confirm-dialog p {
  margin: 0;
  color: var(--muted);
  font-size: 0.9rem;
  line-height: 1.45;
}
.confirm-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 18px;
}

</style>
