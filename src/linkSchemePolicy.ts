import schemes from "../src-tauri/external-link-schemes.json";

const NATIVE_SCHEMES = [schemes.localFile, schemes.deepLink];
const SCHEME_PREFIX = /^([a-z][a-z0-9+.-]*):/i;

function schemePrefix(dest: string): string | undefined {
  // Match the browser's treatment of URL whitespace, even on malformed URLs.
  return SCHEME_PREFIX.exec(dest.replace(/^[\u0000-\u0020]+|[\t\n\r]/g, ""))?.[1].toLowerCase();
}

/** The lowercased URL scheme for consent, reserving single letters for drives. O(bytes). */
export function linkScheme(dest: string): string | null {
  const scheme = schemePrefix(dest);
  return scheme && scheme.length > 1 ? scheme : null;
}

export function refusedLinkScheme(scheme: string): boolean {
  return schemes.refused.includes(scheme);
}

function isDirectExternalScheme(scheme: string): boolean {
  return schemes.web.includes(scheme) || scheme === schemes.mail;
}

/** Only exact, valid scheme names eligible for the existing prompt can be saved.
 * No host, path, prefix, direct-open or refused scheme can become consent. */
export function rememberableLinkScheme(scheme: string): boolean {
  return /^[a-z][a-z0-9+.-]+$/.test(scheme) && !isDirectExternalScheme(scheme)
    && !NATIVE_SCHEMES.includes(scheme) && !refusedLinkScheme(scheme);
}

/** Classify an authored destination before attachment, file or media routing.
 * Relative, web and mail links keep hrefs. File URLs and absolute Windows
 * drives can use their dedicated routes. Custom schemes defer to consent.
 * O(destination bytes), with no I/O. */
export type AnchorDecision = "relative" | "web" | "mail" | "file" | "drive" | "defer" | "drop";
export function anchorDecision(dest: string): AnchorDecision {
  const scheme = schemePrefix(dest);
  if (!scheme) return "relative";
  if (refusedLinkScheme(scheme)) return "drop";
  if (schemes.web.includes(scheme)) return "web";
  if (scheme === schemes.mail) return "mail";
  if (scheme === schemes.localFile && dest.trim().slice(0, scheme.length + 1).toLowerCase() === `${scheme}:`) return "file";
  return /^[a-z]:[/\\]/i.test(dest.trimStart()) ? "drive" : "defer";
}

export function anchorKeepsHref(decision: AnchorDecision): boolean {
  return decision === "relative" || decision === "web" || decision === "mail";
}

/** Reject a destination before browser or native IPC handoff. Registered
 * schemes require exact ASCII case-insensitive consent (D-20261103-S503).
 * Native file URLs and Tine links have dedicated routes. O(URL bytes), no I/O. */
export function checkExternalLink(
  dest: string,
  confirmedScheme: string | undefined,
  platform: "browser" | "native",
): void {
  const rawScheme = platform === "native" ? SCHEME_PREFIX.exec(dest)?.[1] : undefined;
  const scheme = platform === "native" ? rawScheme?.toLowerCase() : linkScheme(dest);
  if (!scheme || scheme.length < 2) throw new Error("unsupported url scheme");
  const nativeOnly = NATIVE_SCHEMES.includes(scheme);
  if (nativeOnly && rawScheme === scheme) return;
  if (refusedLinkScheme(scheme) || nativeOnly) {
    throw new Error(`${scheme}: links are never opened`);
  }
  if (platform === "browser") new URL(dest);
  if (isDirectExternalScheme(scheme)) return;
  const confirmed = confirmedScheme?.replace(/[A-Z]/g, (c) => c.toLowerCase());
  if (confirmed !== scheme) throw new Error(`opening a ${scheme}: link needs your confirmation`);
}

/** Check browser handoffs against the browser's own base for relative URLs.
 * Pass the authored spelling to the handoff after checking its resolved scheme. */
export function checkBrowserHandoff(dest: string, confirmedScheme: string | undefined, base: string): void {
  if (anchorDecision(dest) === "relative") {
    const here = new URL(base);
    const there = new URL(dest, here);
    if (there.protocol === here.protocol && there.host === here.host) return;
    checkExternalLink(there.href, undefined, "browser");
    return;
  }
  checkExternalLink(dest, confirmedScheme, "browser");
}

/** Host-owned marker for an anchor activated by the checked opener. */
export const DEFERRED_HREF = "data-tine-href";

/** Keep, defer or drop a sanitized anchor's href according to the scheme policy. */
export function deferAnchorHref(anchor: Element): void {
  const dest = anchor.getAttribute("href");
  if (dest === null) return;
  const decision = anchorDecision(dest);
  if (anchorKeepsHref(decision)) return;
  anchor.removeAttribute("href");
  if (decision === "drop") return;
  anchor.setAttribute(DEFERRED_HREF, dest);
  anchor.setAttribute("role", "link");
  anchor.setAttribute("tabindex", "0");
}
