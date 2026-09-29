import { createSignal } from "solid-js";
import { backend } from "./backend";
import { pushToast } from "./toasts";
import { persistThemePreference } from "./ui";

const THEME_KEY = "logseq-claude.theme";
export type ThemePreference = "light" | "dark" | "system";

function loadThemePreference(): ThemePreference {
  try {
    const value = localStorage.getItem(THEME_KEY);
    if (value === "dark" || value === "light" || value === "system") return value;
  } catch {
    if (typeof localStorage !== "undefined") pushToast("Could not load theme preference.", "error");
  }
  return "light";
}

let colorSchemeMql: MediaQueryList | null | undefined;
function colorSchemeQuery(): MediaQueryList | null {
  if (colorSchemeMql !== undefined) return colorSchemeMql;
  try {
    colorSchemeMql = typeof window !== "undefined" && typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  } catch { colorSchemeMql = null; }
  return colorSchemeMql;
}

/** Resolve an appearance choice to the active palette. Missing OS signals use
 * Light. Cost: O(1); no I/O or failure. */
export function resolveTheme(pref: ThemePreference): "light" | "dark" {
  return pref === "system" ? (colorSchemeQuery()?.matches ? "dark" : "light") : pref;
}

export const [appearancePreference, setAppearancePreferenceSignal] = createSignal<ThemePreference>(loadThemePreference());
export const [theme, setTheme] = createSignal<"light" | "dark">(resolveTheme(appearancePreference()));
let colorSchemeListening = false;
const onColorSchemeChange = () => { if (appearancePreference() === "system") applyTheme(); };

function syncColorSchemeListener(pref: ThemePreference) {
  const mql = colorSchemeQuery();
  if (!mql || colorSchemeListening === (pref === "system")) return;
  try {
    if (pref === "system") mql.addEventListener("change", onColorSchemeChange);
    else mql.removeEventListener("change", onColorSchemeChange);
    colorSchemeListening = pref === "system";
  } catch { /* A platform without a change listener retains its current resolved theme. */ }
}

/** Apply stored theme at startup; native system-bar update runs async and toasts on failure. */
export function applyTheme() {
  const resolved = resolveTheme(appearancePreference());
  setTheme(resolved);
  document.documentElement.setAttribute("data-theme", resolved);
  void backend().setSystemBarAppearance(resolved === "dark")
    .catch(() => pushToast("Could not update system bar appearance.", "error"));
  syncColorSchemeListener(appearancePreference());
}

/** Persist and apply Light, Dark or System. Storage errors keep this session's
 * choice and show a toast. Cost: O(1) plus one local write and native update. */
export function setAppearancePreference(pref: ThemePreference) {
  setAppearancePreferenceSignal(pref);
  persistThemePreference(THEME_KEY, pref);
  applyTheme();
}
