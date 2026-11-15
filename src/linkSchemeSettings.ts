import { createSignal } from "solid-js";
import { backend } from "./backend";
import { rememberableLinkScheme } from "./linkSchemePolicy";
import { loadPreference, preferenceChangedElsewhere, preferenceRevision, writePreference } from "./preferenceWrites";
import { revisionOwner, writeOwned } from "./owned";
import { msg, t } from "./i18n";

// A JSON array in the existing device-local string accessor: no graph writes or sync.
const KEY = "always_open_link_schemes";
const CHANGED = "link-scheme-consent-changed";
const origin = crypto.randomUUID();
const [schemes, setSchemes] = createSignal<string[]>([]);
export const rememberedLinkSchemes = schemes;
let loaded: Promise<void> | undefined;
let listener: Promise<void> | undefined;

// Capture is a separate WebView. Follow the existing CM6/pinyin event pattern
// so forgetting a scheme retires consent in every already-open window too.
function listenForChanges(): Promise<void> {
  return listener ??= (async () => {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      await listen<{ origin: string }>(CHANGED, event => {
        if (event.payload.origin !== origin) preferenceChangedElsewhere(schemes, () => void initLinkSchemeSettings());
      });
    } catch { /* A browser preview has no native event bus. */ }
  })();
}

function decode(value: string): string[] {
  const parsed: unknown = JSON.parse(value);
  return Array.isArray(parsed)
    ? [...new Set(parsed.filter((scheme): scheme is string => typeof scheme === "string")
      .map(scheme => scheme.toLowerCase()).filter(rememberableLinkScheme))].sort()
    : [];
}

/** Hydrate on startup; revisions keep delayed reads behind later user choices.
 * Invalid settings grant no new consent; parse/read errors preserve the last
 * in-session choice and report (a fresh process starts empty).
 * O(saved scheme bytes), one device read. */
export function initLinkSchemeSettings(): Promise<void> {
  void listenForChanges();
  return loaded = loadPreference(schemes, setSchemes, () => backend().getAppString(KEY, "[]"), decode, t("links-remembered-schemes"));
}

/** A link clicked before startup hydration settles waits for that same read. */
export function loadLinkSchemeSettings(): Promise<void> {
  return loaded ?? initLinkSchemeSettings();
}

/** Apply an explicit choice and serialize device writes with the existing
 * preference queue. A failure rolls back and reports. No I/O on normal opens. */
export function setLinkSchemeRemembered(scheme: string, remember: boolean): void {
  scheme = scheme.toLowerCase();
  if (!rememberableLinkScheme(scheme)) return;
  const next = schemes().filter(value => value !== scheme);
  if (remember) next.push(scheme);
  next.sort();
  writePreference(schemes, setSchemes, next, async value => {
    await writeOwned(revisionOwner(schemes, preferenceRevision(schemes)), backend().setAppString(KEY, JSON.stringify(value)));
    try {
      const { emit } = await import("@tauri-apps/api/event");
      await emit(CHANGED, { origin });
    } catch { /* Persisted; a window without an event bus reads it at startup. */ }
  },
    msg("links-consent-save-failed"));
}
