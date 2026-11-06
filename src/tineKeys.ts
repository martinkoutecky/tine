// Tine's own property keys (D-20261102-PNS, Discussion #825): every property with
// a Tine-specific meaning is `tine.<name>`. The page markers stable wrote in a
// slash form (`tine/favorites:: true` …) are still read, through the ONE alias
// table, `tine_outline::tine_keys::LEGACY_KEY_ALIASES`, which the window reads
// from the wasm parser so the backend and the window can never disagree. Tine
// writes only the canonical spelling. Beta-only keys (`quote::`, `author::`,
// `export::`) were renamed with no alias: reading the old name would keep the
// collision with users' own properties.
import { propertyKeyNorm } from "./propertyKey";
import { tine_key_aliases } from "./render/wasm/lsdoc_wasm.js";

export const FAVORITES_MARKER = "tine.favorites";
export const JOURNAL_SECTIONS_MARKER = "tine.journal-sections";
export const EXPORTS_MARKER = "tine.exports";

let aliases: ReadonlyArray<readonly [string, string]> | null = null;
function legacyAliases(): ReadonlyArray<readonly [string, string]> {
  return (aliases ??= JSON.parse(tine_key_aliases()) as Array<[string, string]>);
}

/** `key` normalized, with a legacy spelling mapped to the key it stands for. */
export function canonicalPropertyKey(key: string): string {
  const norm = propertyKeyNorm(key);
  return legacyAliases().find(([legacy]) => legacy === norm)?.[1] ?? norm;
}

/** Every spelling a reader accepts for `canonical`: itself, then its legacy spellings. */
export function keySpellings(canonical: string): string[] {
  return [canonical, ...legacyAliases().filter(([, c]) => c === canonical).map(([legacy]) => legacy)];
}

/** Whether a page's header properties carry `marker:: true` in any accepted spelling. */
export function hasMarker(properties: readonly (readonly [string, string])[], marker: string): boolean {
  return properties.some(([key, value]) => canonicalPropertyKey(key) === marker && value.trim().toLowerCase() === "true");
}

/** The engine query locating every page marked `marker:: true`, in any accepted spelling. */
export function markerQuery(marker: string): string {
  const terms = keySpellings(marker).map((key) => `(page-property ${key} true)`);
  return terms.length === 1 ? terms[0] : `(or ${terms.join(" ")})`;
}

/** The change keys a marker reader depends on (every accepted spelling). */
export function markerChangeKeys(marker: string): string[] {
  return keySpellings(marker).map((key) => `prop:${key}`);
}
