import { createSignal } from "solid-js";
import { backend } from "../backend";
import { parseThemeManifest, themeManifestCss, themeVersionKey, type ThemeManifest } from "./manifest";
import { pushToast } from "../toasts";

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

interface StoredThemes {
  themes: ThemeManifest[];
  /** Stored entries this build cannot parse (a newer Tine's manifest): carried
   * through every write verbatim, never dropped (I-9). */
  foreign: unknown[];
  /** The whole list was unreadable or rejected: nothing may be persisted over it. */
  rejected: boolean;
}

let storedStatus: "unread" | "ok" | "failed" = "unread";
let foreignEntries: unknown[] = [];
/** Install/uninstall are read-modify-write over one stored list; run them one at a time. */
let writeQueue: Promise<unknown> = Promise.resolve();

function parseStoredThemes(text: string): StoredThemes {
  try {
    const value: unknown = JSON.parse(text);
    if (!Array.isArray(value) || value.length > MAX_INSTALLED_THEMES) {
      pushToast("Installed theme settings are invalid.", "error");
      return { themes: [], foreign: [], rejected: true };
    }
    const themes: ThemeManifest[] = [];
    const foreign: unknown[] = [];
    for (const candidate of value) {
      try { themes.push(parseThemeManifest(candidate)); } catch { foreign.push(candidate); }
    }
    if (foreign.length) pushToast("Some installed themes could not be loaded.", "error");
    return { themes, foreign, rejected: false };
  } catch {
    pushToast("Installed theme settings could not be parsed.", "error");
    return { themes: [], foreign: [], rejected: true };
  }
}

function managed(manifests: ThemeManifest[]): InstalledTheme[] {
  return manifests.map((manifest) => {
    const key = themeVersionKey(manifest);
    return { id: key, key, manifest, css: themeManifestCss(manifest) };
  });
}

async function persist(themes: InstalledTheme[]) {
  await backend().setAppString(
    STORAGE_KEY,
    JSON.stringify([...themes.map((theme) => theme.manifest), ...foreignEntries]),
  );
}

/** Install revocations before the first await, then read up to 32 manifests
 * from device settings. Read or whole-list parse failure toasts, leaves an
 * empty list and REFUSES later install/uninstall writes (they retry this read
 * first) so the stored list is never replaced by a partial view; entries this
 * build cannot parse are kept and re-written verbatim. No write here. O(manifests
 * and CSS size) plus one backend read. */
export async function initThemePackages(initialRevocations: ReadonlySet<string> = new Set()): Promise<void> {
  // Seed before the first await so a selected installed theme can never be
  // restored through an empty startup revocation window.
  applyThemeRevocations(initialRevocations);
  let text = "[]";
  let readFailed = false;
  try { text = await backend().getAppString(STORAGE_KEY, "[]"); }
  catch { readFailed = true; pushToast("Could not load installed themes.", "error"); }
  const stored = readFailed ? { themes: [], foreign: [], rejected: true } : parseStoredThemes(text);
  storedStatus = stored.rejected ? "failed" : "ok";
  foreignEntries = stored.foreign;
  setInstalledThemes(managed(stored.themes));
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

const storedListTrusted = () => storedStatus === "ok";

/** Run one read-modify-write of the stored list after every earlier one. A list
 * that failed to load is re-read first; if it still cannot be trusted the write
 * is refused rather than persisted over it. */
function serialized<T>(work: () => Promise<T>): Promise<T> {
  const run = writeQueue.then(async () => {
    if (!storedListTrusted()) {
      await initThemePackages(revokedThemeVersions());
      if (!storedListTrusted()) {
        throw new Error("installed themes could not be read; not overwriting the stored list");
      }
    }
    return work();
  });
  writeQueue = run.catch(() => undefined);
  return run;
}

export async function installThemePackage(value: unknown): Promise<InstalledTheme> {
  const manifest = parseThemeManifest(value);
  const key = themeVersionKey(manifest);
  if (themeVersionIsRevoked(key)) {
    throw new Error("this theme version was revoked by the signed registry");
  }
  return serialized(async () => {
    const current = installedThemes();
    if (!current.some((theme) => theme.key === key) && current.length + foreignEntries.length >= MAX_INSTALLED_THEMES) {
      throw new Error(`at most ${MAX_INSTALLED_THEMES} theme versions may be installed`);
    }
    const next = [...current.filter((theme) => theme.key !== key), ...managed([manifest])];
    await persist(next);
    setInstalledThemes(next);
    return next[next.length - 1];
  });
}

export async function uninstallThemePackage(key: string): Promise<void> {
  await serialized(async () => {
    const next = installedThemes().filter((theme) => theme.key !== key);
    await persist(next);
    setInstalledThemes(next);
  });
}
