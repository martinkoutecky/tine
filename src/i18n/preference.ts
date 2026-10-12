// The UI language preference (tine#107): a device preference like spellcheck,
// persisted in tine-settings.json under `ui_language` ("" = the system's), never
// in the graph or localStorage. Every webview reads it before it mounts, so the
// first painted frame is already in the chosen language (G3); a change in one
// window is broadcast to the others.
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { preferenceReadCurrent, preferenceRevision, seedPreference, supersedePreference, writePreference } from "../preferenceWrites";
import { pushToast } from "../toasts";
import { loadLocale, msg, negotiateLocale, showLocale, SOURCE_LOCALE } from "./index";

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

// One owner for the UI language (review CODE-1 F1/F2, CODE-2 A/B): every change
// (the startup read, its fallback, a broadcast from another window, the user's
// choice, a write rollback) starts an operation, and only the latest operation
// may show a locale, fall back, seed the committed value or tell native code. A
// superseded operation's load, failure or late read is inert, whatever order
// they finish in. Startup's mount barrier follows the latest operation, not its own.
type Operation = { id: number; done: Promise<void>; superseded: Promise<void>; supersede: () => void };
let current: Operation | null = null;
let operation = 0;
const begin = (): number => {
  current?.supersede();
  let supersede!: () => void;
  const superseded = new Promise<void>((resolve) => { supersede = resolve; });
  current = { id: ++operation, done: Promise.resolve(), superseded, supersede };
  return operation;
};
const owns = (mine: number) => mine === operation;
/** Record `work` as operation `mine`'s; returns it. */
function track(mine: number, work: Promise<void>): Promise<void> {
  if (current && current.id === mine) current.done = work;
  return work;
}
/** Settles once the latest operation has, following each supersession. */
async function settled(): Promise<void> {
  for (;;) {
    const op = current;
    if (!op) return;
    await Promise.race([op.done.catch(() => {}), op.superseded]);
    if (current === op) return;
  }
}

/** Show `choice`'s locale for operation `mine`: its catalog first (shared with
 * any concurrent load of it), English if that fails, nothing if superseded. */
async function activate(choice: string, mine: number): Promise<void> {
  if (!owns(mine)) return;
  setPreferenceSignal(choice);
  let tag = negotiateLocale(choice, systemLanguages());
  try {
    await loadLocale(tag);
  } catch {
    if (!owns(mine)) return;
    console.error("i18n: catalog failed to load; staying in English");
    tag = SOURCE_LOCALE;
  }
  if (!owns(mine)) return;
  showLocale(tag);
  // Native text (the tray) follows the resolved locale; native code never negotiates.
  void backend().setUiLocale(tag)
    .catch(() => console.error("i18n: the native shell did not take the language"));
}

type Read = { ok: true; stored: string } | { ok: false };

/** The startup read's result, for operation `mine` if it is still the latest and
 * no write of this window raced it. */
async function land(read: Read, mine: number, revision: number): Promise<void> {
  if (!owns(mine) || !preferenceReadCurrent(preference, revision)) return;
  if (!read.ok) {
    pushToast(msg("toast-language-load-failed"), "error");
    await activate("", mine);
    return;
  }
  await activate(read.stored, mine);
  if (owns(mine)) seedPreference(preference);
}

/** Read the stored choice and show its locale. Await before mounting a webview:
 * it settles once the chosen catalog is showing. Only the settings read is
 * bounded (READ_BOUND_MS): a slower read shows the system's language, then the
 * stored choice when it lands, unless anything changed the language meanwhile.
 * A failed read toasts and keeps the system's language. One settings read. */
export async function startUiLanguage(): Promise<void> {
  // Subscribe first, so a change broadcast during the read is never missed.
  await listenForChanges();
  const mine = begin();
  const revision = preferenceRevision(preference);
  const reading: Promise<Read> = backend().getAppString(KEY, "")
    .then((stored) => ({ ok: true as const, stored }), () => ({ ok: false as const }));
  void track(mine, (async () => {
    const first = await Promise.race([
      reading,
      new Promise<null>((resolve) => setTimeout(() => resolve(null), READ_BOUND_MS)),
    ]);
    if (first) return land(first, mine, revision);
    await activate("", mine);
    void reading.then((late) => land(late, mine, revision));
  })());
  await settled();
}

async function listenForChanges(): Promise<void> {
  try {
    const { listen } = await import("@tauri-apps/api/event");
    await listen<string>(BROADCAST, (event) => {
      // Another window persisted this choice (or this one did: its echo keeps
      // every window on the last write). It is committed, and an older local
      // write's failure must not roll back over it (review CODE-2 B).
      supersedePreference(preference, event.payload);
      // Always a new operation, even for the value already showing: it retires an
      // older startup read that would otherwise land after it (review CODE-1 F2).
      const mine = begin();
      void track(mine, activate(event.payload, mine));
    });
  } catch {
    // Not in Tauri (tests, the published export): one window only.
  }
}

/** Choose the UI language (`""` = the system's): applies now, persists, tells the other windows. */
export function setUiLanguage(choice: string): void {
  writePreference(preference, (next) => { const mine = begin(); void track(mine, activate(next, mine)); }, choice,
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
