// The UI language preference (tine#107): a device preference like spellcheck,
// persisted in tine-settings.json under `ui_language` ("" = the system's), never
// in the graph or localStorage. Every webview reads it before it mounts, so the
// first painted frame is already in the chosen language (G3); a change in one
// window is broadcast to the others.
import { createSignal } from "solid-js";
import { backend } from "../backend";
import { ownedWhen, readOwned } from "../owned";
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

// One owner for the UI language (review CODE-1 F1/F2, CODE-2 A/B, CODE-3 B2):
// every change (the startup read, its fallback, a broadcast from another window,
// the user's choice, a failed save's re-read) starts an operation, and only the
// latest operation may show a locale, fall back or tell native code. A superseded
// operation's load, failure or late read is inert, whatever order they finish
// in. Startup's mount barrier follows the latest operation, not its own. The
// committed value is the settings file alone: nothing here mirrors it, so a
// failed save rolls back by reading it again.
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
/** Record `work` as operation `mine`'s. Its failure is logged here, so the
 * mount barrier and fire-and-forget callers never see a rejection. */
function track(mine: number, work: Promise<void>): Promise<void> {
  const logged = work.catch(() => console.error("i18n: a language change failed"));
  if (current && current.id === mine) current.done = logged;
  return logged;
}
/** Settles once the latest operation has, following each supersession. */
async function settled(): Promise<void> {
  for (;;) {
    const op = current;
    if (!op) return;
    await Promise.race([op.done, op.superseded]);
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

/** Read the stored choice for operation `mine`: `null` once it is no longer
 * the latest (a retired read is inert, failure included). */
const readStored = (mine: number): Promise<Read | null> =>
  readOwned(ownedWhen(() => owns(mine)), backend().getAppString(KEY, "")).then(
    (result) => result.kind === "stale" ? null : { ok: true as const, stored: result.value },
    () => ({ ok: false as const }),
  );

/** A settings read's result, for operation `mine` if it is still the latest. */
async function land(read: Read | null, mine: number): Promise<void> {
  if (!read || !owns(mine)) return;
  if (!read.ok) {
    pushToast(msg("toast-language-load-failed"), "error");
    await activate("", mine);
    return;
  }
  await activate(read.stored, mine);
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
  const reading = readStored(mine);
  void track(mine, (async () => {
    const first = await Promise.race([
      reading,
      new Promise<"slow">((resolve) => setTimeout(() => resolve("slow"), READ_BOUND_MS)),
    ]);
    if (first !== "slow") return land(first, mine);
    await activate("", mine);
    void reading.then((late) => land(late, mine));
  })());
  await settled();
}

async function listenForChanges(): Promise<void> {
  try {
    const { listen } = await import("@tauri-apps/api/event");
    await listen<string>(BROADCAST, (event) => {
      // Another window persisted this choice (or this one did: its echo keeps
      // every window on the last write).
      // Always a new operation, even for the value already showing: it retires an
      // older startup read that would otherwise land after it (review CODE-1 F2).
      const mine = begin();
      void track(mine, activate(event.payload, mine));
    });
  } catch {
    // Not in Tauri (tests, the published export): one window only.
  }
}

/** This window's saves, in call order. */
let saves: Promise<void> = Promise.resolve();

/** Choose the UI language (`""` = the system's): applies now, persists, tells the
 * other windows. A failed save toasts and, unless something newer changed the
 * language meanwhile, shows what the settings file still holds (CODE-3 B2). */
export function setUiLanguage(choice: string): void {
  const mine = begin();
  void track(mine, activate(choice, mine));
  saves = saves.then(async () => {
    try {
      await backend().setAppString(KEY, choice);
    } catch {
      pushToast(msg("toast-language-save-failed"), "error");
      if (!owns(mine)) return;
      const again = begin();
      void track(again, readStored(again).then((read) => land(read, again)));
      return;
    }
    try {
      const { emit } = await import("@tauri-apps/api/event");
      await emit(BROADCAST, choice);
    } catch {
      // Not in Tauri.
    }
  });
}
