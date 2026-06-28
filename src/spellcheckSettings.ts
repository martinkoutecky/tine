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
  apply();
}
