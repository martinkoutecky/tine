// The UI language preference (tine#107): a device preference like spellcheck,
// persisted in tine-settings.json under `ui_language` ("" = the system's), never
// in the graph or localStorage. Every webview reads it before it mounts, so the
// first painted frame is already in the chosen language (G3); a change in one
// window is broadcast to the others.
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { writePreference } from "../preferenceWrites";
import { activeLocale, negotiateLocale, setActiveLocale, SOURCE_LOCALE } from "./index";

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
  } catch (error) {
    console.error("i18n: catalog failed to load; staying in English", error);
    await setActiveLocale(SOURCE_LOCALE);
  }
  // Native text (the tray) follows the resolved locale; native code never negotiates.
  void backend().setUiLocale(activeLocale()).catch(() => {});
}

/** Read the stored choice and activate its locale. Await before mounting a webview. */
export async function startUiLanguage(): Promise<void> {
  const stored = await Promise.race([
    backend().getAppString(KEY, "").catch(() => ""),
    new Promise<string>((resolve) => setTimeout(() => resolve(""), READ_BOUND_MS)),
  ]);
  await apply(stored);
  void listenForChanges();
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
