import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { PerkIndexStatus, PerkSearchResult, Project, ProjectPatch, Viewer } from "../types";
import { useSettings } from "./useSettings";

const projects = ref<Project[]>([]);
const viewers = ref<Viewer[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
let libraryLoaded = false;
let libraryLoadPromise: Promise<void> | null = null;

export function useLibrary() {
  const { settings } = useSettings();

  async function loadLibrary(force = false) {
    if (!force && libraryLoaded) {
      return;
    }

    if (!force && libraryLoadPromise) {
      return libraryLoadPromise;
    }

    libraryLoadPromise = (async () => {
      try {
        loading.value = true;
        error.value = null;
        projects.value = await invoke<Project[]>("get_library");
        libraryLoaded = true;
      } catch (e: any) {
        error.value = String(e);
      } finally {
        loading.value = false;
        libraryLoadPromise = null;
      }
    })();

    return libraryLoadPromise;
  }

  async function takeLibraryMigrationNotice(): Promise<string | null> {
    return invoke<string | null>("take_library_migration_notice");
  }

  async function loadViewers() {
    try {
      viewers.value = await invoke<Viewer[]>("get_viewers");
    } catch (e: any) {
      console.error("Failed to load viewers:", e);
    }
  }

  async function addProject(filePath: string): Promise<Project> {
    const project = await invoke<Project>("add_project", { filePath });
    projects.value.push(project);
    return project;
  }

  async function startDownloadProject(
    url: string,
    maxProjectSizeMb: number,
    downloadIncludedIccPlusViewer: boolean,
  ): Promise<string> {
    return invoke<string>("start_download_project", {
      url,
      maxProjectSizeMb,
      downloadIncludedIccPlusViewer,
    });
  }

  async function startDownloadCatalogEntry(
    taskId: string,
    websiteUrl: string,
    zipUrl: string,
    projectName: string,
    maxProjectSizeMb: number,
  ): Promise<string> {
    return invoke<string>("start_download_catalog_entry", {
      taskId,
      websiteUrl,
      zipUrl,
      projectName,
      maxProjectSizeMb,
    });
  }

  async function startOverwriteCatalogEntry(
    taskId: string,
    projectId: string,
    websiteUrl: string,
    zipUrl: string,
    projectName: string,
    maxProjectSizeMb: number,
  ): Promise<string> {
    return invoke<string>("start_overwrite_catalog_entry", {
      taskId,
      projectId,
      websiteUrl,
      zipUrl,
      projectName,
      maxProjectSizeMb,
    });
  }

  async function applyOversizeProjectAction(
    id: string,
    strategy: "keep-separate" | "compress" | "do-nothing",
  ): Promise<Project> {
    return invoke<Project>("apply_oversize_project_action", { id, strategy });
  }

  async function startApplyOversizeProjectAction(
    id: string,
    strategy: "keep-separate" | "compress" | "do-nothing",
  ): Promise<string> {
    return invoke<string>("start_apply_oversize_project_action", { id, strategy });
  }

  async function addProjectsBulk(filePaths: string[]) {
    const added: Project[] = [];
    for (const fp of filePaths) {
      try {
        const p = await invoke<Project>("add_project", { filePath: fp });
        projects.value.push(p);
        added.push(p);
      } catch (e) {
        console.error("Failed to add:", fp, e);
      }
    }
    return added;
  }

  async function removeProject(id: string) {
    await invoke("remove_project", { id });
    projects.value = projects.value.filter((p) => p.id !== id);
  }

  async function removeProjectFromDisk(id: string) {
    await invoke("remove_project_from_disk", { id });
    projects.value = projects.value.filter((p) => p.id !== id);
  }

  async function clearLibrary() {
    await invoke("clear_library");
    projects.value = [];
    libraryLoaded = true;
  }

  async function compressLibraryCoverImages(): Promise<number> {
    const changed = await invoke<number>("compress_library_cover_images");
    if (changed > 0) {
      await loadLibrary(true);
    }
    return changed;
  }

  async function updateProject(id: string, patch: ProjectPatch): Promise<Project> {
    const updated = await invoke<Project>("update_project", { id, patch });
    const idx = projects.value.findIndex((p) => p.id === id);
    if (idx !== -1) projects.value[idx] = updated;
    return updated;
  }

  function setProjectFavoriteLocally(id: string, favorite: boolean) {
    const idx = projects.value.findIndex((p) => p.id === id);
    if (idx === -1) {
      return;
    }

    projects.value[idx] = {
      ...projects.value[idx],
      favorite,
    };
  }

  async function setProjectFavorite(id: string, favorite: boolean): Promise<Project> {
    const idx = projects.value.findIndex((p) => p.id === id);
    const previousFavorite = idx === -1 ? null : projects.value[idx].favorite;

    setProjectFavoriteLocally(id, favorite);

    try {
      const updated = await invoke<Project>("set_project_favorite", { id, favorite });
      if (idx !== -1) {
        projects.value[idx] = updated;
      }
      return updated;
    } catch (error) {
      if (previousFavorite !== null) {
        setProjectFavoriteLocally(id, previousFavorite);
      }
      throw error;
    }
  }

  function setProjectViewerPreferenceLocally(id: string, viewerId: string) {
    const idx = projects.value.findIndex((p) => p.id === id);
    if (idx === -1) {
      return;
    }

    projects.value[idx] = {
      ...projects.value[idx],
      viewer_preference: viewerId,
    };
  }

  async function openViewer(project: Project, viewerId: string) {
    await invoke("open_viewer_window", {
      projectId: project.id,
      viewerId,
      projectName: project.name,
      cheatsEnabled: settings.value.cheatsEnabled,
    });

    if (project.viewer_preference !== viewerId) {
      setProjectViewerPreferenceLocally(project.id, viewerId);
      void invoke<Project>("set_project_viewer_preference", {
        id: project.id,
        viewerPreference: viewerId,
      }).catch((error) => {
        console.error("Failed to save project viewer preference:", error);
      });
    }
  }

  async function scanFolder(folder: string): Promise<string[]> {
    return invoke<string[]>("scan_folder", { folder });
  }

  async function startScanFolder(folder: string): Promise<string> {
    return invoke<string>("start_scan_folder", { folder });
  }

  async function getPerkIndexStatus(): Promise<PerkIndexStatus> {
    return invoke<PerkIndexStatus>("get_perk_index_status");
  }

  async function startPerkIndexTask(
    includeImages: boolean,
    forceRebuild: boolean,
  ): Promise<string> {
    return invoke<string>("start_perk_index_task", { includeImages, forceRebuild });
  }

  async function syncPerkIndex(includeImages: boolean): Promise<PerkIndexStatus> {
    return invoke<PerkIndexStatus>("sync_perk_index", { includeImages });
  }

  async function rebuildPerkIndex(includeImages: boolean): Promise<PerkIndexStatus> {
    return invoke<PerkIndexStatus>("rebuild_perk_index", { includeImages });
  }

  async function searchPerks(
    query: string,
    limit = 100,
    offset = 0,
  ): Promise<PerkSearchResult[]> {
    return invoke<PerkSearchResult[]>("search_perks", { query, limit, offset });
  }

  const allTags = computed(() => {
    const set = new Set<string>();
    projects.value.forEach((p) => p.tags.forEach((t) => set.add(t)));
    return [...set].sort();
  });

  return {
    projects,
    viewers,
    loading,
    error,
    loadLibrary,
    takeLibraryMigrationNotice,
    loadViewers,
    addProject,
    startDownloadProject,
    startDownloadCatalogEntry,
    startOverwriteCatalogEntry,
    applyOversizeProjectAction,
    startApplyOversizeProjectAction,
    addProjectsBulk,
    removeProject,
    removeProjectFromDisk,
    clearLibrary,
    compressLibraryCoverImages,
    updateProject,
    setProjectFavorite,
    openViewer,
    scanFolder,
    startScanFolder,
    getPerkIndexStatus,
    startPerkIndexTask,
    syncPerkIndex,
    rebuildPerkIndex,
    searchPerks,
    allTags,
  };
}
