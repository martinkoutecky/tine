// Device-local spellcheck preferences (persisted in tine-settings.json via the
// generic app_bool/app_string backend for atomic, WebView-independent state).
// Read at startup by initSpellcheckSettings(); the
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
import { writePreference, seedPreference, preferenceRevision, preferenceReadCurrent } from "./preferenceWrites";
import { pushToast } from "./toasts";

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
  void backend().applySpellcheck(enabled(), parseLanguages(languages()))
    .catch(() => pushToast("Could not apply spellcheck settings.", "error"));
}

/** Apply immediately; native spellcheck runs without awaiting. Queue a device
 * write; failure rolls back and toasts. Native failure also toasts. Return
 * confirms neither asynchronous result. O(1) plus native and settings calls. */
export function setSpellcheckEnabled(on: boolean): void {
  writePreference(enabled, (next) => { setEnabledSig(next); apply(); }, on,
    (next) => backend().setAppBool(KEY_ENABLED, next), "spellcheck preference");
}

/** Apply immediately; native spellcheck runs without awaiting. Queue a device
 * write; failure rolls back and toasts. Native failure also toasts. Return
 * confirms neither asynchronous result. O(1) plus native and settings calls. */
export function setSpellcheckLanguages(value: string): void {
  writePreference(languages, (next) => { setLanguagesSig(next); apply(); }, value,
    (next) => backend().setAppString(KEY_LANGS, next), "spellcheck languages");
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

/** Read device preferences into this WebView unless superseded by local writes.
 * Failed reads toast and resolve. Dictionary refresh and native application
 * run without awaiting; native failures toast. O(1) backend and native calls. */
export async function initSpellcheckSettings(): Promise<void> {
  const enabledRevision = preferenceRevision(enabled);
  const languageRevision = preferenceRevision(languages);
  try {
    const value = await backend().getAppBool(KEY_ENABLED, true);
    if (preferenceReadCurrent(enabled, enabledRevision)) { setEnabledSig(value); seedPreference(enabled); }
  } catch {
    pushToast("Could not load spellcheck preference.", "error");
  }
  try {
    const value = await backend().getAppString(KEY_LANGS, "");
    if (preferenceReadCurrent(languages, languageRevision)) { setLanguagesSig(value); seedPreference(languages); }
  } catch {
    pushToast("Could not load spellcheck languages.", "error");
  }
  void loadDictionaries();
  apply();
}
