import type { Viewer } from "./types";

const ICC_PLUS_VIEWER_IDS = new Set(["icc-plus", "icc2-plus"]);
const ICC_PLUS_VIEWER_NAMES = new Set(["icc plus", "icc2 plus"]);

function isIccPlusViewer(viewer: Viewer): boolean {
  return ICC_PLUS_VIEWER_IDS.has(viewer.id)
    || ICC_PLUS_VIEWER_NAMES.has(viewer.name.trim().toLowerCase());
}

export function resolveViewerId(
  viewers: Viewer[],
  preferredViewerId: string | null | undefined,
  defaultViewerId: string | null | undefined,
): string | null {
  const preferredViewer = viewers.find((viewer) => viewer.id === preferredViewerId);
  if (preferredViewer) {
    return preferredViewer.id;
  }

  const defaultViewer = viewers.find((viewer) => viewer.id === defaultViewerId);
  if (defaultViewer) {
    return defaultViewer.id;
  }

  const iccPlusViewer = viewers.find(isIccPlusViewer);
  if (iccPlusViewer) {
    return iccPlusViewer.id;
  }

  return viewers[0]?.id ?? null;
}