<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(defineProps<{
  label: string;
  value: number;
  details?: string;
}>(), {
  details: "",
});

const clampedPercent = computed(() => Math.max(0, Math.min(100, Math.round(props.value))));
</script>

<template>
  <div class="progress-wrap">
    <div class="progress-meta">
      <span>{{ label }}</span>
      <span>{{ clampedPercent }}%</span>
    </div>
    <div class="progress-bar">
      <div class="progress-fill" :style="{ width: `${clampedPercent}%` }" />
    </div>
    <div v-if="details" class="progress-sub">{{ details }}</div>
  </div>
</template>

<style scoped>
.progress-wrap {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.progress-meta {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: 0.82rem;
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
</style>
