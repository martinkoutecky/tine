import { createSignal } from "solid-js";
import { backend } from "../backend";
import { parseThemeManifest, themeManifestCss, themeVersionKey, type ThemeManifest } from "./manifest";

const STORAGE_KEY = "theme.packages.v1";
const MAX_INSTALLED_THEMES = 32;

export interface InstalledTheme {
  id: string;
  key: string;
  manifest: ThemeManifest;
  css: string;
}

const [installedThemes, setInstalledThemes] = createSignal<InstalledTheme[]>([]);
const [revokedThemeVersions, setRevokedThemeVersions] = createSignal<ReadonlySet<string>>(new Set());
export { installedThemes, revokedThemeVersions };

function parseStoredThemes(text: string): ThemeManifest[] {
  try {
    const value: unknown = JSON.parse(text);
    if (!Array.isArray(value) || value.length > MAX_INSTALLED_THEMES) return [];
    const themes: ThemeManifest[] = [];
    for (const candidate of value) {
      try { themes.push(parseThemeManifest(candidate)); } catch {}
    }
    return themes;
  } catch {
    return [];
  }
}

function managed(manifests: ThemeManifest[]): InstalledTheme[] {
  return manifests.map((manifest) => {
    const key = themeVersionKey(manifest);
    return { id: key, key, manifest, css: themeManifestCss(manifest) };
  });
}

async function persist(themes: InstalledTheme[]) {
  await backend().setAppString(STORAGE_KEY, JSON.stringify(themes.map((theme) => theme.manifest)));
}

export async function initThemePackages(initialRevocations: ReadonlySet<string> = new Set()): Promise<void> {
  // Seed before the first await so a selected installed theme can never be
  // restored through an empty startup revocation window.
  applyThemeRevocations(initialRevocations);
  let text = "[]";
  try { text = await backend().getAppString(STORAGE_KEY, "[]"); } catch {}
  setInstalledThemes(managed(parseStoredThemes(text)));
}

export function installedThemeByKey(key: string): InstalledTheme | undefined {
  if (revokedThemeVersions().has(key)) return undefined;
  return installedThemes().find((theme) => theme.key === key);
}

export function themeVersionIsRevoked(key: string): boolean {
  return revokedThemeVersions().has(key);
}

export function applyThemeRevocations(revoked: ReadonlySet<string>): void {
  setRevokedThemeVersions(new Set(revoked));
}

export async function installThemePackage(value: unknown): Promise<InstalledTheme> {
  const manifest = parseThemeManifest(value);
  const key = themeVersionKey(manifest);
  if (themeVersionIsRevoked(key)) {
    throw new Error("this theme version was revoked by the signed registry");
  }
  const current = installedThemes();
  if (!current.some((theme) => theme.key === key) && current.length >= MAX_INSTALLED_THEMES) {
    throw new Error(`at most ${MAX_INSTALLED_THEMES} theme versions may be installed`);
  }
  const next = [...current.filter((theme) => theme.key !== key), ...managed([manifest])];
  await persist(next);
  setInstalledThemes(next);
  return next[next.length - 1];
}

export async function uninstallThemePackage(key: string): Promise<void> {
  const next = installedThemes().filter((theme) => theme.key !== key);
  await persist(next);
  setInstalledThemes(next);
}
