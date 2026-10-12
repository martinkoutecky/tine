// The UI language preference (tine#107): a device preference like spellcheck,
// persisted in tine-settings.json under `ui_language` ("" = the system's), never
// in the graph or localStorage. Every webview reads it before it mounts, so the
// first painted frame is already in the chosen language (G3); a change in one
// window is broadcast to the others.
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { preferenceReadCurrent, preferenceRevision, seedPreference, writePreference } from "../preferenceWrites";
import { pushToast } from "../toasts";
import { activeLocale, msg, negotiateLocale, setActiveLocale, SOURCE_LOCALE } from "./index";

const KEY = "ui_language";
const BROADCAST = "ui-language-changed";
/** A slow settings read must not hold the first frame: past this, the system's language. */
const READ_BOUND_MS = 500;

const [preference, setPreferenceSignal] = createSignal("");

/** Reactive: the stored choice, `""` for the system default. */
export const uiLanguagePreference = preference;

/** The system's language list, most preferred first. */
export function systemLanguages(): readonly string[] {
  if (typeof navigator === "undefined") return [];
  return navigator.languages?.length ? navigator.languages : navigator.language ? [navigator.language] : [];
}

async function apply(choice: string): Promise<void> {
  setPreferenceSignal(choice);
  const tag = negotiateLocale(choice, systemLanguages());
  try {
    await setActiveLocale(tag);
  } catch {
    console.error("i18n: catalog failed to load; staying in English");
    await setActiveLocale(SOURCE_LOCALE);
  }
  // Native text (the tray) follows the resolved locale; native code never negotiates.
  void backend().setUiLocale(activeLocale())
    .catch(() => console.error("i18n: the native shell did not take the language"));
}

/** Read the stored choice and activate its locale. Await before mounting a webview.
 * A read slower than READ_BOUND_MS mounts in the system's language and applies the
 * stored choice when it lands, unless the user chose meanwhile; a failed read keeps
 * the system's language and toasts. One settings read; O(1) otherwise. */
export async function startUiLanguage(): Promise<void> {
  void listenForChanges();
  const revision = preferenceRevision(preference);
  let landed = false;
  const landing = backend().getAppString(KEY, "").then(async (stored) => {
    landed = true;
    if (!preferenceReadCurrent(preference, revision)) return;
    await apply(stored);
    seedPreference(preference);
  }, async () => {
    landed = true;
    pushToast(msg("toast-language-load-failed"), "error");
    await apply("");
  });
  await Promise.race([landing, new Promise<void>((resolve) => setTimeout(resolve, READ_BOUND_MS))]);
  if (!landed) await apply("");
}

async function listenForChanges(): Promise<void> {
  try {
    const { listen } = await import("@tauri-apps/api/event");
    await listen<string>(BROADCAST, (event) => {
      if (event.payload !== preference()) void apply(event.payload);
    });
  } catch {
    // Not in Tauri (tests, the published export): one window only.
  }
}

/** Choose the UI language (`""` = the system's): applies now, persists, tells the other windows. */
export function setUiLanguage(choice: string): void {
  writePreference(preference, (next) => { void apply(next); }, choice,
    async (next) => {
      await backend().setAppString(KEY, next);
      try {
        const { emit } = await import("@tauri-apps/api/event");
        await emit(BROADCAST, next);
      } catch {
        // Not in Tauri.
      }
    }, "language preference");
}
