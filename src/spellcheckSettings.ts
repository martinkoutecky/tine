// Device-local spellcheck preferences (persisted in tine-settings.json via the
// generic app_bool/app_string backend, so they survive a restart — WebKitGTK
// localStorage does not). Read at startup by initSpellcheckSettings(); the
// `spellcheckEnabled` signal gates the editor <textarea spellcheck> attribute,
// and `applySpellcheck` pushes the toggle + languages onto the native WebKitGTK
// spell checker live (no restart — unlike Logseq, which needs a relaunch).
//
// Defaults match Logseq: spellcheck ON, and an EMPTY language list ⇒ follow the
// OS locale. Beyond Logseq: list several locale codes (e.g. "en_US, cs_CZ") and
// WebKitGTK checks every dictionary at once, so words valid in ANY listed
// language aren't flagged — proper bilingual editing. (Each language needs its
// hunspell dictionary installed; a missing one is silently ignored.)

import { createSignal } from "solid-js";
import { backend } from "./backend";

const KEY_ENABLED = "spellcheck_enabled";
const KEY_LANGS = "spellcheck_languages";

const [enabled, setEnabledSig] = createSignal(true);
const [languages, setLanguagesSig] = createSignal("");
const [dictionaries, setDictionaries] = createSignal<string[]>([]);

/** Reactive: locale codes of the spell-check dictionaries installed on this
 *  machine (from the backend), so the UI can offer a pick-list. */
export const spellcheckDictionaries = dictionaries;

/** Reactive: is spellcheck on? Gates the editor `<textarea spellcheck>`. ON by
 *  default, like Logseq. */
export const spellcheckEnabled = enabled;
/** Reactive: the raw languages string the user typed (e.g. "en_US, cs_CZ"). Empty
 *  ⇒ follow the OS locale, like Logseq. */
export const spellcheckLanguages = languages;

/** Parse a user language string into locale codes (comma / space / semicolon). */
export function parseLanguages(s: string): string[] {
  return s.split(/[\s,;]+/).map((t) => t.trim()).filter(Boolean);
}

function apply(): void {
  void backend().applySpellcheck(enabled(), parseLanguages(languages())).catch(() => {});
}

export function setSpellcheckEnabled(on: boolean): void {
  setEnabledSig(on);
  void backend().setAppBool(KEY_ENABLED, on).catch(() => {});
  apply();
}

export function setSpellcheckLanguages(value: string): void {
  setLanguagesSig(value);
  void backend().setAppString(KEY_LANGS, value).catch(() => {});
  apply();
}

/** Is this dictionary code currently selected? */
export function isLanguageSelected(code: string): boolean {
  return parseLanguages(languages()).includes(code);
}

/** Tick/untick one dictionary in the selection (preserving the others). */
export function toggleSpellcheckLanguage(code: string, on: boolean): void {
  const set = new Set(parseLanguages(languages()));
  if (on) set.add(code);
  else set.delete(code);
  setSpellcheckLanguages([...set].join(", "));
}

/** Human-readable name for a locale code via the platform's own language data —
 *  e.g. "en_US" → "American English", "cs_CZ" → "Czech (Czechia)". Falls back to
 *  the raw code if Intl can't resolve it. */
export function languageDisplayName(code: string): string {
  const bcp = code.replace(/_/g, "-");
  try {
    const dn = new Intl.DisplayNames([navigator.language || "en"], { type: "language" });
    return dn.of(bcp) ?? code;
  } catch {
    return code;
  }
}

/** (Re)load the installed dictionaries from the backend. Cheap; call on startup
 *  and from a "Rescan" button (the user may install a dictionary mid-session). */
export async function loadDictionaries(): Promise<void> {
  try {
    setDictionaries(await backend().listSpellcheckDictionaries());
  } catch {
    setDictionaries([]);
  }
}

/** Load persisted prefs at startup and push them onto the webview. The Rust setup
 *  already applied them once from the same file; re-applying is idempotent and
 *  also sets this webview-context's signals (e.g. the separate capture window). */
export async function initSpellcheckSettings(): Promise<void> {
  try {
    setEnabledSig(await backend().getAppBool(KEY_ENABLED, true));
  } catch {
    /* default on */
  }
  try {
    setLanguagesSig(await backend().getAppString(KEY_LANGS, ""));
  } catch {
    /* default: OS locale */
  }
  void loadDictionaries();
  apply();
}
