/** Existing outbound-link policy, shared by opening and consent storage.
 * Native platform.rs independently rechecks every OS handoff. Unlike OG
 * og-1.0.0 electron/window.cljs open-default-app!, Tine asks for custom schemes
 * unless this device remembers explicit consent (D-20261103-S503). */
const DIRECT_SCHEMES = new Set(["http", "https", "mailto", "file", "tine"]);
const NEVER_OPENED_SCHEMES = new Set(["javascript", "vbscript", "data", "blob", "about"]);

/** RFC 3986 scheme, lowercased; a single letter is a Windows drive. O(bytes). */
export function linkScheme(dest: string): string | null {
  const match = /^([a-z][a-z0-9+.-]+):/i.exec(dest);
  return match ? match[1].toLowerCase() : null;
}

export function refusedLinkScheme(scheme: string): boolean {
  return NEVER_OPENED_SCHEMES.has(scheme);
}

/** Only exact, valid scheme names eligible for the existing prompt can be saved.
 * No host, path, prefix, direct-open or refused scheme can become consent. */
export function rememberableLinkScheme(scheme: string): boolean {
  return linkScheme(`${scheme}:`) === scheme && !DIRECT_SCHEMES.has(scheme) && !refusedLinkScheme(scheme);
}
