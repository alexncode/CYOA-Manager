export interface Project {
  id: string;
  name: string;
  description: string;
  cover_image: string | null;
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

export type SortKey = "name" | "date_added";
export type Theme = "light" | "dark" | "system";
