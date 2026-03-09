<script setup lang="ts">
defineProps<{
  finalSizeMb: number;
  limitMb: number;
  busy: boolean;
  status: string;
}>();

const emit = defineEmits<{
  (e: "choose", strategy: "keep-separate" | "compress" | "do-nothing"): void;
}>();
</script>

<template>
  <div class="oversize-content">
    <h3>Project is too large</h3>
    <p>Final size is {{ finalSizeMb }} MB, limit is {{ limitMb }} MB.</p>
    <div class="oversize-actions">
      <button
        class="oversize-action-btn oversize-action-primary"
        :disabled="busy"
        @click="emit('choose', 'keep-separate')"
      >
        Keep images separate
      </button>
      <button
        class="oversize-action-btn oversize-action-accent"
        :disabled="busy"
        @click="emit('choose', 'compress')"
      >
        Attempt compression (can take very long)
      </button>
      <button
        class="oversize-action-btn oversize-action-muted"
        :disabled="busy"
        @click="emit('choose', 'do-nothing')"
      >
        Do nothing
      </button>
    </div>
    <div v-if="busy" class="oversize-running">{{ status || "Applying action…" }}</div>
  </div>
</template>

<style scoped>
.oversize-content h3 {
  margin: 0 0 6px;
  font-size: 0.96rem;
}

.oversize-content p {
  margin: 0;
  color: var(--muted);
  font-size: 0.86rem;
  line-height: 1.45;
}

.oversize-actions {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 10px;
}

.oversize-action-btn {
  width: 100%;
  min-height: 38px;
  border-radius: 8px;
  border: 1px solid transparent;
  font-size: 0.85rem;
  font-weight: 600;
  cursor: pointer;
  transition: filter 0.15s ease, opacity 0.15s ease;
}

.oversize-action-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.oversize-action-primary {
  background: linear-gradient(135deg, #3f86ff, #2f68d6);
  color: #f6fbff;
}

.oversize-action-accent {
  background: linear-gradient(135deg, #2db38a, #248e6d);
  color: #f3fffa;
}

.oversize-action-muted {
  background: #4f5665;
  color: #e8ebf2;
}

.oversize-action-btn:not(:disabled):hover {
  filter: brightness(1.06);
}

.oversize-running {
  margin-top: 8px;
  font-size: 0.82rem;
  color: var(--muted);
}
</style>
