export interface Project {
  id: string;
  name: string;
  description: string;
  cover_image: string | null;
  source_url?: string | null;
  file_path: string;
  viewer_preference: string | null;
  date_added: string;
  tags: string[];
  file_missing: boolean;
}

export interface ProjectPatch {
  name?: string;
  description?: string;
  /** empty string clears the cover */
  cover_image?: string;
  viewer_preference?: string;
  tags?: string[];
  /** re-link a broken card */
  file_path?: string;
}

export interface Viewer {
  id: string;
  name: string;
}

export interface PerkIndexStatus {
  ready: boolean;
  needsReindex: boolean;
  indexedProjects: number;
  totalProjects: number;
  perkCount: number;
  imagesEnabled: boolean;
  lastIndexedAt: string | null;
}

export interface PerkSearchResult {
  projectId: string;
  projectName: string;
  rowId: string;
  rowTitle: string;
  objectId: string;
  title: string;
  description: string;
  points: string | null;
  addons: string[];
  imagePath: string | null;
}

export interface CatalogEntry {
  name: string;
  date: string;
  website: string;
  link: string;
  author?: string;
  universe?: string;
  importer?: string;
  type?: string;
  pov?: string;
  length?: string;
  tags?: string[];
  description?: string;
}

export type SortKey = "name" | "date_added";
export type Theme = "light" | "dark" | "system";
export type OversizeDefaultAction = "ask" | "keep-separate" | "compress";
export type OversizeActionStrategy = "keep-separate" | "compress" | "do-nothing";
