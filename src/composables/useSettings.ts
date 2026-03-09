import { ref, watch } from "vue";
import type { Theme } from "../types";

const STORAGE_KEY = "cyoa-manager-settings";

interface Settings {
  defaultViewer: string | null;
  theme: Theme;
  cheatsEnabled: boolean;
  downloadSizeLimitMb: number;
}

function load(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return { ...defaults(), ...JSON.parse(raw) };
  } catch {
    /* ignore */
  }
  return defaults();
}

function defaults(): Settings {
  return {
    defaultViewer: null,
    theme: "system",
    cheatsEnabled: true,
    downloadSizeLimitMb: 200,
  };
}

const settings = ref<Settings>(load());

watch(
  settings,
  (val) => localStorage.setItem(STORAGE_KEY, JSON.stringify(val)),
  { deep: true }
);

export function useSettings() {
  function applyTheme() {
    const theme = settings.value.theme;
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    const dark = theme === "dark" || (theme === "system" && prefersDark);
    document.documentElement.classList.toggle("dark", dark);
  }

  return { settings, applyTheme };
}
