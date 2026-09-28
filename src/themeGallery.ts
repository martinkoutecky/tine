import { createSignal } from "solid-js";
import { backend } from "./backend";
import { CUSTOM_CSS_STYLE_ID, LS_SHIM_STYLE_ID, ensureLsShimStyle } from "./lsShim";
import { galleryThemeById, galleryThemes } from "./styles/themes";
import { installedThemeByKey } from "./themes/manager";
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";

export const THEME_GALLERY_STYLE_ID = "tine-theme";
const KEY = "theme.gallery";

const [selectedId, setSelectedId] = createSignal("");

export const selectedGalleryTheme = selectedId;
export { galleryThemes };

export function ensureThemeStyle(): HTMLStyleElement | null {
  if (typeof document === "undefined") return null;

  const shim = ensureLsShimStyle();
  let el = document.getElementById(THEME_GALLERY_STYLE_ID) as HTMLStyleElement | null;
  if (el && el.tagName !== "STYLE") {
    el.remove();
    el = null;
  }
  if (!el) {
    el = document.createElement("style");
    el.id = THEME_GALLERY_STYLE_ID;
  }

  const custom = document.getElementById(CUSTOM_CSS_STYLE_ID);
  const afterShim = shim?.parentNode === document.head ? shim.nextSibling : null;
  if (shim?.parentNode === document.head && afterShim !== el) {
    document.head.insertBefore(el, afterShim);
  } else if (custom?.parentNode === document.head) {
    document.head.insertBefore(el, custom);
  } else if (el.parentNode !== document.head) {
    document.head.appendChild(el);
  }

  const shimAgain = document.getElementById(LS_SHIM_STYLE_ID);
  const customAgain = document.getElementById(CUSTOM_CSS_STYLE_ID);
  if (shimAgain?.parentNode === document.head && shimAgain.nextSibling !== el) {
    document.head.insertBefore(el, shimAgain.nextSibling);
  }
  if (customAgain?.parentNode === document.head) {
    const nodes = Array.from(document.head.childNodes);
    if (nodes.indexOf(el) > nodes.indexOf(customAgain)) {
      document.head.insertBefore(el, customAgain);
    }
  }

  return el;
}

function applyThemeLocally(id: string): void {
  const theme = id ? galleryThemeById(id) ?? installedThemeByKey(id) : undefined;
  const nextId = theme?.id ?? "";
  setSelectedId(nextId);
  const el = ensureThemeStyle();
  if (el) el.textContent = theme?.css ?? "";
}

/** Apply bundled or installed non-revoked CSS immediately. Unknown or revoked
 * IDs clear selection and persist empty ID. Write failure rolls back and toasts.
 * O(theme lookup and CSS size) plus one backend write. */
export function applyTheme(id: string): void {
  const theme = id ? galleryThemeById(id) ?? installedThemeByKey(id) : undefined;
  writePreference(selectedId, applyThemeLocally, theme?.id ?? "",
    (next) => backend().setAppString(KEY, next), "gallery theme");
}

/** Load saved theme; unknown IDs clear selection. Read failure toasts and
 * resolves, and later user selection wins. O(theme lookup and CSS size). */
export async function initThemeGallery(): Promise<void> {
  ensureThemeStyle();
  const revision = preferenceRevision(selectedId);
  let id = "";
  try {
    id = await backend().getAppString(KEY, "");
  } catch {
    id = "";
    pushToast("Could not load gallery theme.", "error");
  }
  if (preferenceReadCurrent(selectedId, revision)) {
    applyThemeLocally(id);
    seedPreference(selectedId);
  }
}

if (typeof window !== "undefined") {
  window.__tineApplyTheme = applyTheme;
}
