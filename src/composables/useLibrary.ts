import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Project, ProjectPatch, Viewer } from "../types";

const projects = ref<Project[]>([]);
const viewers = ref<Viewer[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

export function useLibrary() {
  async function loadLibrary() {
    try {
      loading.value = true;
      error.value = null;
      projects.value = await invoke<Project[]>("get_library");
    } catch (e: any) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
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

  async function startDownloadProject(url: string): Promise<string> {
    return invoke<string>("start_download_project", { url });
  }

  async function startDownloadCatalogEntry(taskId: string, websiteUrl: string, zipUrl: string, projectName: string): Promise<string> {
    return invoke<string>("start_download_catalog_entry", { taskId, websiteUrl, zipUrl, projectName });
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

  async function clearLibrary() {
    await invoke("clear_library");
    projects.value = [];
  }

  async function updateProject(id: string, patch: ProjectPatch): Promise<Project> {
    const updated = await invoke<Project>("update_project", { id, patch });
    const idx = projects.value.findIndex((p) => p.id === id);
    if (idx !== -1) projects.value[idx] = updated;
    return updated;
  }

  async function openViewer(project: Project, viewerId: string) {
    if (project.viewer_preference !== viewerId) {
      project = await updateProject(project.id, { viewer_preference: viewerId });
    }
    await invoke("open_viewer_window", {
      projectId: project.id,
      viewerId,
      projectName: project.name,
    });
  }

  async function scanFolder(folder: string): Promise<string[]> {
    return invoke<string[]>("scan_folder", { folder });
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
    loadViewers,
    addProject,
    startDownloadProject,
    startDownloadCatalogEntry,
    addProjectsBulk,
    removeProject,
    clearLibrary,
    updateProject,
    openViewer,
    scanFolder,
    allTags,
  };
}
