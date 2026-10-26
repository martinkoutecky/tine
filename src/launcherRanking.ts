import { createSignal } from "solid-js";
import { backend } from "./backend";
import type { ObjectiveMatchClass } from "./types";
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";
import { advanceRevision, latestOwner, readOwned, revisionOwner, writeOwned } from "./owned";
import { activeLocale, msg, render } from "./i18n";

const PINYIN_KEY = "pinyin-page-matching";
const PINYIN_CHANGED = "pinyin-page-matching-changed";
const pinyinWindow = crypto.randomUUID();
let pinyinListener: Promise<void> | undefined;
function listenForPinyinChanges(): Promise<void> {
  return pinyinListener ??= (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      await listen<{ origin: string }>(PINYIN_CHANGED, event => {
        if (event.payload.origin === pinyinWindow) return;
        advanceRevision(pinyinEnabled);
        void initPinyinMatchingSetting();
      });
    } catch { /* Published pages have a single window. */ }
  })();
}
/** Default follows Chinese UI or primary system language; reset uses this too. */
export function pinyinDefault(ui = activeLocale(), system = typeof navigator === "undefined" ? "" : navigator.language): boolean {
  return ui.toLowerCase().startsWith("zh") || system.toLowerCase().startsWith("zh");
}
const [pinyinEnabled, applyPinyinEnabled] = createSignal(pinyinDefault());
export const pinyinMatchingEnabled = pinyinEnabled;

/** Device preference; a newer local write wins over startup reads, and a newer
 * refresh (Capture show, change broadcast) retires an older one still in flight. */
export async function initPinyinMatchingSetting(): Promise<void> {
  await listenForPinyinChanges();
  const revision = preferenceRevision(pinyinEnabled);
  let superseded = () => false;
  try {
    const owner = latestOwner(pinyinEnabled, "refresh", revisionOwner(pinyinEnabled, revision));
    superseded = () => !owner();
    const ui = await readOwned(owner, backend().getAppString("ui_language", ""));
    if (ui.kind === "stale") return;
    const fallback = pinyinDefault(ui.value || activeLocale());
    const loaded = await readOwned(owner, backend().getAppBool(PINYIN_KEY, fallback));
    if (loaded.kind === "stale") return;
    const value = loaded.value;
    if (preferenceReadCurrent(pinyinEnabled, revision)) {
      applyPinyinEnabled(value); seedPreference(pinyinEnabled);
    }
  } catch {
    if (superseded()) return;
    if (preferenceReadCurrent(pinyinEnabled, revision)) applyPinyinEnabled(pinyinDefault());
    pushToast(msg("toast-pinyin-load-failed"), "error");
  }
}

export function setPinyinMatchingEnabled(value: boolean): void {
  writePreference(pinyinEnabled, applyPinyinEnabled, value,
    async next => {
      await writeOwned(revisionOwner(pinyinEnabled, preferenceRevision(pinyinEnabled)), backend().setAppBool(PINYIN_KEY, next));
      try {
        const { emit } = await import("@tauri-apps/api/event");
        await emit(PINYIN_CHANGED, { origin: pinyinWindow });
      } catch { /* No native event bus in published pages. */ }
    }, render(msg("settings-pinyin-label")), () => pushToast(msg("toast-pinyin-save-failed"), "error"));
}

export function resetPinyinMatchingSetting(): void {
  setPinyinMatchingEnabled(pinyinDefault());
}
void initPinyinMatchingSetting();

const SETTING_KEY = "adaptive-launcher-ranking";
const VERSION = 1;
const MAX_RECORDS = 400;
const MAX_AGE_MS = 180 * 24 * 60 * 60 * 1000;
const HALF_LIFE_MS = 30 * 24 * 60 * 60 * 1000;
const OBJECTIVE_CLASS_RANK: Record<ObjectiveMatchClass, number> = {
  exact: 0,
  prefix: 1,
  substring: 2,
  fuzzy: 3,
  transliteration: 4,
  body_evidence: 5,
};

interface ActivationRecord {
  query: string;
  identity: string;
  count: number;
  last: number;
}

interface RankingStore {
  version: number;
  records: ActivationRecord[];
}

export interface LauncherRankable {
  adaptiveClass: ObjectiveMatchClass;
  adaptiveIdentity: string;
  adaptiveFavorite?: boolean;
}

const [enabled, setEnabledSignal] = createSignal(true);
export const launcherRankingEnabled = enabled;

/** Load device setting (default ON). Read failure toasts and resolves; a newer
 * local write wins. O(1) plus backend read. */
export async function initLauncherRankingSetting(): Promise<void> {
  const revision = preferenceRevision(enabled);
  try {
    const value = await backend().getAppBool(SETTING_KEY, true);
    if (preferenceReadCurrent(enabled, revision)) { setEnabledSignal(value); seedPreference(enabled); }
  } catch {
    if (preferenceReadCurrent(enabled, revision)) setEnabledSignal(true);
    pushToast(msg("launcher-ranking-could-not-load-adaptive-launcher-ranking-preference"), "error");
  }
}

/** Apply now and queue a device-local write. Failure rolls back and toasts;
 * return does not confirm persistence. O(1) plus backend write. */
export function setLauncherRankingEnabled(value: boolean): void {
  writePreference(enabled, setEnabledSignal, value, (next) => backend().setAppBool(SETTING_KEY, next), "adaptive launcher ranking");
}

function graphKey(root: string): string {
  // FNV-1a keeps graph paths out of the localStorage key while remaining
  // deterministic and graph-scoped. The data itself contains no graph text.
  let hash = 0x811c9dc5;
  for (const char of root) {
    hash ^= char.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193);
  }
  return `tine.launcher-ranking.v1.${(hash >>> 0).toString(16)}`;
}

function normalizeQuery(query: string): string {
  return query.trim().toLowerCase().normalize("NFC");
}

function read(root: string, storage: Storage = localStorage): RankingStore {
  try {
    const parsed = JSON.parse(storage.getItem(graphKey(root)) ?? "null") as RankingStore | null;
    if (parsed?.version === VERSION && Array.isArray(parsed.records)) return parsed;
  } catch {
    // Corrupt or unavailable device-local history is equivalent to no history.
  }
  return { version: VERSION, records: [] };
}

function write(root: string, store: RankingStore, storage: Storage = localStorage): void {
  try {
    storage.setItem(graphKey(root), JSON.stringify(store));
  } catch {
    // Ranking is optional; storage failure must never block navigation.
  }
}

function bounded(records: ActivationRecord[], now: number): ActivationRecord[] {
  return records
    .filter((record) => record.count > 0 && now - record.last <= MAX_AGE_MS)
    .sort((a, b) => b.last - a.last)
    .slice(0, MAX_RECORDS);
}

export function recordLauncherActivation(
  root: string,
  query: string,
  identity: string,
  now = Date.now(),
  storage: Storage = localStorage,
): void {
  if (!enabled() || !root || !query.trim() || !identity) return;
  const normalized = normalizeQuery(query);
  const store = read(root, storage);
  const existing = store.records.find(
    (record) => record.query === normalized && record.identity === identity,
  );
  if (existing) {
    existing.count = Math.min(255, existing.count + 1);
    existing.last = now;
  } else {
    store.records.push({ query: normalized, identity, count: 1, last: now });
  }
  store.records = bounded(store.records, now);
  write(root, store, storage);
}

function frecency(record: ActivationRecord | undefined, now: number): number {
  // One activation deliberately has no ordering effect: an accidental open
  // cannot teach the launcher. Repetition is required.
  if (!record || record.count < 2) return 0;
  const decay = Math.pow(0.5, Math.max(0, now - record.last) / HALF_LIFE_MS);
  return Math.log2(record.count) * decay;
}

export function rankLauncherItems<T extends LauncherRankable>(
  root: string,
  query: string,
  items: readonly T[],
  now = Date.now(),
  storage: Storage = localStorage,
): T[] {
  if (!enabled() || !root || !query.trim() || items.length < 2) return [...items];
  const normalized = normalizeQuery(query);
  const records = read(root, storage).records.filter((record) => record.query === normalized);
  const byIdentity = new Map(records.map((record) => [record.identity, record]));
  return items
    .map((item, index) => ({ item, index, score: frecency(byIdentity.get(item.adaptiveIdentity), now) }))
    .sort((a, b) => {
      // Objective classes never cross. Compare their semantic rank directly;
      // using original indices here would make the comparator non-transitive if
      // one producer ever interleaved two bands.
      if (a.item.adaptiveClass !== b.item.adaptiveClass) {
        return OBJECTIVE_CLASS_RANK[a.item.adaptiveClass] - OBJECTIVE_CLASS_RANK[b.item.adaptiveClass];
      }
      // The backend's deterministic order is the final stable tie breaker inside
      // a class; favorites and frecency cannot promote a weaker objective match.
      return Number(!!b.item.adaptiveFavorite) - Number(!!a.item.adaptiveFavorite)
        || b.score - a.score
        || a.index - b.index;
    })
    .map(({ item }) => item);
}

export function resetLauncherRanking(root: string, storage: Storage = localStorage): void {
  if (!root) return;
  try {
    storage.removeItem(graphKey(root));
  } catch {
    // optional state
  }
}

void initLauncherRankingSetting();
