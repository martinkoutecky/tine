import { createSignal } from "solid-js";
import { backend } from "./backend";
import { CUSTOM_CSS_STYLE_ID, LS_SHIM_STYLE_ID, ensureLsShimStyle } from "./lsShim";
import { galleryThemeById, galleryThemes } from "./styles/themes";
import { installedThemeByKey } from "./themes/manager";
import type { ThemePresentation } from "./themes/manifest";
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";
import { readOwned, revisionOwner } from "./owned";

export const THEME_GALLERY_STYLE_ID = "tine-theme";
/** Pre-composition single-id key; read only when no composition is stored. */
const LEGACY_KEY = "theme.gallery";
/** Persisted `{ style, colors }` JSON (master 670cf75bb): the presentation
 * source and the palette source are chosen independently. */
const COMPOSITION_KEY = "theme.composition.v1";

type ThemeComposition = { style: string; colors: string };

const [composition, setComposition] = createSignal<ThemeComposition>({ style: "", colors: "" });
const [selectedPresentation, setSelectedPresentation] = createSignal<ThemePresentation>({});

/** Theme whose API 0.2 presentation is active ("" = Tine's default). */
export const selectedThemeStyle = () => composition().style;
/** Theme whose color tokens are active ("" = Tine's default palette). */
export const selectedThemeColors = () => composition().colors;
/** The active host-owned presentation presets (ADR 0059). */
export const selectedThemePresentation = selectedPresentation;
/** The single selected theme as the Settings gallery shows it: its colors. */
export const selectedGalleryTheme = selectedThemeColors;
export { galleryThemes };

const PRESENTATION_ATTRIBUTES = {
  contentTypography: "data-theme-content-typography",
  journalHeader: "data-theme-journal-header",
  todayTaskSummary: "data-theme-today-task-summary",
} as const;

function applyPresentation(presentation: ThemePresentation): void {
  setSelectedPresentation(presentation);
  if (typeof document === "undefined") return;
  for (const [key, attribute] of Object.entries(PRESENTATION_ATTRIBUTES)) {
    const value = presentation[key as keyof ThemePresentation];
    if (value === undefined || value === "default" || value === "hidden") {
      document.documentElement.removeAttribute(attribute);
    } else {
      document.documentElement.setAttribute(attribute, value);
    }
  }
}

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

function themeById(id: string) {
  return id ? galleryThemeById(id) ?? installedThemeByKey(id) : undefined;
}

/** Bundled, installed and non-revoked ids only; a style source must also
 * declare a presentation. Anything else resolves to "" (Tine's default). */
function resolveComposition(style: string, colors: string): ThemeComposition {
  const styleTheme = themeById(style);
  const hasPresentation = !!styleTheme && "manifest" in styleTheme
    && Object.keys(styleTheme.manifest.presentation ?? {}).length > 0;
  return { style: hasPresentation ? styleTheme.id : "", colors: themeById(colors)?.id ?? "" };
}

function applyCompositionLocally(next: ThemeComposition): void {
  const resolved = resolveComposition(next.style, next.colors);
  setComposition(resolved);
  const styleTheme = themeById(resolved.style);
  applyPresentation(styleTheme && "manifest" in styleTheme ? styleTheme.manifest.presentation ?? {} : {});
  const el = ensureThemeStyle();
  if (el) el.textContent = themeById(resolved.colors)?.css ?? "";
}

/** Apply a composition now and persist it through the device preference
 * path; a failed latest write rolls back and toasts. O(theme lookup + CSS). */
function selectComposition(style: string, colors: string): void {
  writePreference(composition, applyCompositionLocally, resolveComposition(style, colors),
    (next) => backend().setAppString(COMPOSITION_KEY, JSON.stringify(next)), "gallery theme");
}

/** Compatibility preset: one theme's presentation (if any) and colors together.
 * Unknown or revoked ids select Tine's default. */
export function applyTheme(id: string): void {
  selectComposition(id, id);
}

/** Change only the presentation source, keeping the palette. */
export function applyThemeStyle(id: string): void {
  selectComposition(id, selectedThemeColors());
}

/** Change only the palette source, keeping the presentation. */
export function applyThemeColors(id: string): void {
  selectComposition(selectedThemeStyle(), id);
}

/** Drop `id` from whichever roles it fills (e.g. before uninstalling it). */
export function clearThemeSelection(id: string): void {
  const { style, colors } = composition();
  selectComposition(style === id ? "" : style, colors === id ? "" : colors);
}

/** Re-resolve the current selection after revocations or package changes. */
export function reapplyThemeSelection(): void {
  const { style, colors } = composition();
  selectComposition(style, colors);
}

function decodeComposition(text: string): ThemeComposition | null {
  try {
    const value: unknown = JSON.parse(text);
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    const { style, colors } = value as Record<string, unknown>;
    return typeof style === "string" && typeof colors === "string" ? { style, colors } : null;
  } catch {
    return null;
  }
}

/** Load the saved composition (or the pre-composition single id as both
 * roles); unknown ids resolve to the default. Read failure toasts and
 * resolves, and a later user selection wins. O(theme lookup and CSS size). */
export async function initThemeGallery(): Promise<void> {
  ensureThemeStyle();
  const revision = preferenceRevision(composition);
  // Current while no selection has advanced the revision or is still pending.
  const owner = revisionOwner(composition, revision, () => preferenceReadCurrent(composition, revision));
  let stored: ThemeComposition = { style: "", colors: "" };
  try {
    const composed = await readOwned(owner, backend().getAppString(COMPOSITION_KEY, ""));
    if (composed.kind === "stale") return;
    const decoded = decodeComposition(composed.value);
    if (decoded) stored = decoded;
    else {
      const legacy = await readOwned(owner, backend().getAppString(LEGACY_KEY, ""));
      if (legacy.kind === "stale") return;
      stored = { style: legacy.value, colors: legacy.value };
    }
  } catch {
    pushToast("Could not load gallery theme.", "error");
  }
  if (owner()) {
    applyCompositionLocally(stored);
    seedPreference(composition);
  }
}

if (typeof window !== "undefined") {
  window.__tineApplyTheme = applyTheme;
}
