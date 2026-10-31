import { msg } from "./i18n";
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
 * source and the palette source are chosen independently. API 0.3 adds a third,
 * independent source, `typography` (GH #760); it is written only when chosen, so
 * a composition without typography is byte-identical to before. */
const COMPOSITION_KEY = "theme.composition.v1";

type ThemeComposition = { style: string; colors: string; typography: string };

const EMPTY: ThemeComposition = { style: "", colors: "", typography: "" };
const [composition, setComposition] = createSignal<ThemeComposition>(EMPTY);
const [selectedPresentation, setSelectedPresentation] = createSignal<ThemePresentation>({});

/** Theme whose API 0.2 presentation is active ("" = Tine's default). */
export const selectedThemeStyle = () => composition().style;
/** Theme whose color tokens are active ("" = Tine's default palette). */
export const selectedThemeColors = () => composition().colors;
/** Installed theme whose API 0.3 fonts and line height are active ("" = Tine's own). */
export const selectedThemeTypography = () => composition().typography;
/** The active host-owned presentation presets (ADR 0059). */
export const selectedThemePresentation = selectedPresentation;
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
function resolveComposition(style: string, colors: string, typography: string): ThemeComposition {
  const styleTheme = themeById(style);
  const hasPresentation = !!styleTheme && "manifest" in styleTheme
    && Object.keys(styleTheme.manifest.presentation ?? {}).length > 0;
  const typographyTheme = themeById(typography);
  const hasTypography = !!typographyTheme && "manifest" in typographyTheme && typographyTheme.typographyCss !== "";
  return {
    style: hasPresentation ? styleTheme.id : "",
    colors: themeById(colors)?.id ?? "",
    typography: hasTypography ? typographyTheme.id : "",
  };
}

/** The composition as stored: `typography` only when one is chosen. */
function encodeComposition(value: ThemeComposition): string {
  const { typography, ...rest } = value;
  return JSON.stringify(typography ? value : rest);
}

function applyCompositionLocally(next: ThemeComposition): void {
  const resolved = resolveComposition(next.style, next.colors, next.typography);
  setComposition(resolved);
  const styleTheme = themeById(resolved.style);
  applyPresentation(styleTheme && "manifest" in styleTheme ? styleTheme.manifest.presentation ?? {} : {});
  const el = ensureThemeStyle();
  if (el) {
    // Typography is host-generated CSS from a validated manifest (src/themes/typography.ts),
    // placed in the same element so custom.css, which follows it, keeps the last word.
    const typographyTheme = themeById(resolved.typography);
    const typographyCss = typographyTheme && "typographyCss" in typographyTheme ? typographyTheme.typographyCss : "";
    el.textContent = [themeById(resolved.colors)?.css ?? "", typographyCss].filter(Boolean).join("\n\n");
  }
}

/** Apply a composition now and persist it through the device preference
 * path; a failed latest write rolls back and toasts. O(theme lookup + CSS). */
function selectComposition(style: string, colors: string, typography: string): void {
  writePreference(composition, applyCompositionLocally, resolveComposition(style, colors, typography),
    (next) => backend().setAppString(COMPOSITION_KEY, encodeComposition(next)), "gallery theme");
}

/** Compatibility preset: one theme's presentation (if any) and colors together.
 * Unknown or revoked ids select Tine's default. Typography is its own choice
 * and stays as it was. */
export function applyTheme(id: string): void {
  selectComposition(id, id, selectedThemeTypography());
}

/** Change only the presentation source, keeping the palette. */
export function applyThemeStyle(id: string): void {
  selectComposition(id, selectedThemeColors(), selectedThemeTypography());
}

/** Change only the palette source, keeping the presentation. */
export function applyThemeColors(id: string): void {
  selectComposition(selectedThemeStyle(), id, selectedThemeTypography());
}

/** Change only the typography source (API 0.3 fonts and line height), keeping
 * the palette and presentation. "" returns to Tine's own typography. */
export function applyThemeTypography(id: string): void {
  selectComposition(selectedThemeStyle(), selectedThemeColors(), id);
}

/** Drop `id` from whichever roles it fills (e.g. before uninstalling it). */
export function clearThemeSelection(id: string): void {
  const { style, colors, typography } = composition();
  selectComposition(style === id ? "" : style, colors === id ? "" : colors, typography === id ? "" : typography);
}

/** Re-resolve the current selection after revocations or package changes. */
export function reapplyThemeSelection(): void {
  const { style, colors, typography } = composition();
  selectComposition(style, colors, typography);
}

function decodeComposition(text: string): ThemeComposition | null {
  try {
    const value: unknown = JSON.parse(text);
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    const { style, colors, typography } = value as Record<string, unknown>;
    if (typeof style !== "string" || typeof colors !== "string") return null;
    return { style, colors, typography: typeof typography === "string" ? typography : "" };
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
  let stored: ThemeComposition = EMPTY;
  try {
    const composed = await readOwned(owner, backend().getAppString(COMPOSITION_KEY, ""));
    if (composed.kind === "stale") return;
    const decoded = decodeComposition(composed.value);
    if (decoded) stored = decoded;
    else {
      const legacy = await readOwned(owner, backend().getAppString(LEGACY_KEY, ""));
      if (legacy.kind === "stale") return;
      stored = { style: legacy.value, colors: legacy.value, typography: "" };
    }
  } catch {
    pushToast(msg("theme-gallery-could-not-load-gallery-theme"), "error");
  }
  if (owner()) {
    applyCompositionLocally(stored);
    seedPreference(composition);
  }
}

if (typeof window !== "undefined") {
  window.__tineApplyTheme = applyTheme;
}
