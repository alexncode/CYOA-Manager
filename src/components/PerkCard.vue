<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, useTemplateRef, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { PerkSearchResult } from "../types";

const props = defineProps<{
  perk: PerkSearchResult;
}>();

const emit = defineEmits<{
  (e: "open-project", projectId: string): void;
}>();

const cardRef = useTemplateRef<HTMLElement>("cardRef");
const imageSrc = ref<string | null>(null);
const imageVisible = ref(false);
let imageObserver: IntersectionObserver | null = null;

watch(
  () => [props.perk.imagePath, imageVisible.value] as const,
  async (nextImagePath) => {
    if (!nextImagePath[0] || !nextImagePath[1]) {
      imageSrc.value = null;
      return;
    }

    try {
      imageSrc.value = await invoke<string | null>("resolve_local_image_src", {
        imagePath: nextImagePath[0],
      });
    } catch {
      imageSrc.value = null;
    }
  },
  { immediate: true },
);

onMounted(() => {
  if (!cardRef.value) {
    return;
  }

  imageObserver = new IntersectionObserver(
    (entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        imageVisible.value = true;
        imageObserver?.disconnect();
        imageObserver = null;
      }
    },
    { rootMargin: "300px 0px" },
  );
  imageObserver.observe(cardRef.value);
});

onBeforeUnmount(() => {
  imageObserver?.disconnect();
  imageObserver = null;
});
</script>

<template>
  <article ref="cardRef" class="perk-card">
    <img v-if="imageSrc" class="perk-image" :src="imageSrc" :alt="perk.title" loading="lazy" />
    <div class="perk-body">
      <div class="perk-meta">
        <button class="perk-project" title="Open project" @click="emit('open-project', perk.projectId)">
          {{ perk.projectName }}
        </button>
        <p v-html="perk.rowTitle" title="Row Title" class="perk-row"></p>
      </div>
      <h3 class="perk-title" v-html="perk.title" />
      <p v-if="perk.points" class="perk-points" v-html="perk.points"></p>
      <div v-if="perk.description" class="perk-description" v-html="perk.description" />
      <div v-if="perk.addons.length" class="perk-addons">
        <div v-for="(addon, index) in perk.addons" :key="`${perk.objectId}-${index}`" class="perk-addon" v-html="addon" />
      </div>
    </div>
  </article>
</template>

<style scoped>
.perk-card {
  display: flex;
  flex-direction: column;
  background: var(--card-bg);
  border: 1px solid var(--border);
  border-radius: 14px;
  overflow: hidden;
  user-select: text;
}

.perk-image {
  width: 100%;
  height: auto;
  display: block;
  background: var(--cover-placeholder);
}

.perk-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
  user-select: text;
}

.perk-meta {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: 0.76rem;
  color: var(--muted);
}

.perk-title {
  margin: 0;
  font-size: 1rem;
  line-height: 1.3;
  text-align: center;
}

.perk-description,
.perk-row {
  margin: 0;
  line-height: 1.45;
  color: var(--text);
  white-space: pre-wrap;
}

.perk-title :deep(*),
.perk-description :deep(*),
.perk-addon :deep(*) {
  user-select: text;
}

.perk-row {
  color: var(--muted);
  font-size: 0.82rem;
}

.perk-addons {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.perk-addon {
  background: color-mix(in srgb, var(--card-bg) 78%, #000 22%);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 10px 12px;
  font-size: 0.82rem;
  line-height: 1.45;
  white-space: pre-wrap;
}

.perk-points {
  color: var(--accent);
  font-weight: 600;
  font-size: 0.82rem;
  text-align: center;
  margin: -4px 0 0;
}

.perk-project {
  font-weight: 600;
  background: none;
  border: none;
  padding: 0;
  color: var(--accent);
  cursor: pointer;
  text-align: left;
}

.perk-project:hover {
  text-decoration: underline;
}
</style>