import schemes from "../src-tauri/external-link-schemes.json";

const NATIVE_SCHEMES = [schemes.localFile, schemes.deepLink];
const WEB_SCHEMES = schemes.webPrefixes.map((prefix) => prefix.split(":")[0]);
const MAIL_SCHEME = schemes.mailPrefix.slice(0, -1);
const DIRECT_OPEN_PREFIXES = [...schemes.webPrefixes, schemes.mailPrefix];
// One-letter prefixes reserve a browser href but name a drive in graph content.
const SCHEME_PREFIX = /^(?:([a-z][a-z0-9+.-]+)|[a-z]):/i;

function schemePrefix(dest: string): RegExpExecArray | null {
  // Match the browser's treatment of URL whitespace, even on malformed URLs.
  return SCHEME_PREFIX.exec(dest.replace(/^[\u0000-\u0020]+|[\t\n\r]/g, ""));
}

/** The lowercased URL scheme for consent, reserving single letters for drives. O(bytes). */
export function linkScheme(dest: string): string | null {
  return schemePrefix(dest)?.[1]?.toLowerCase() ?? null;
}

export function refusedLinkScheme(scheme: string): boolean {
  return schemes.refused.includes(scheme);
}

function isWebOrMailScheme(scheme: string): boolean {
  return WEB_SCHEMES.includes(scheme) || scheme === MAIL_SCHEME;
}

/** Saved consent is for valid registered app scheme names. */
export function rememberableLinkScheme(scheme: string): boolean {
  return /^[a-z][a-z0-9+.-]+$/.test(scheme) && !isWebOrMailScheme(scheme)
    && !NATIVE_SCHEMES.includes(scheme) && !refusedLinkScheme(scheme);
}

/** Classify an authored destination before attachment, file or media routing.
 * Relative, web and mail links keep hrefs. Drive prefixes enter the scheme-less
 * routes, with browser navigation suppressed. Custom schemes defer to consent.
 * O(destination bytes), with no I/O. */
export type AnchorDecision = "relative" | "web" | "mail" | "file" | "drive" | "defer" | "drop";
export function anchorDecision(dest: string): AnchorDecision {
  const prefix = schemePrefix(dest);
  if (!prefix) return "relative";
  const scheme = prefix[1]?.toLowerCase();
  if (!scheme) return "drive";
  if (refusedLinkScheme(scheme)) return "drop";
  if (WEB_SCHEMES.includes(scheme)) return "web";
  if (scheme === MAIL_SCHEME) return "mail";
  if (scheme === schemes.localFile && dest.trim().slice(0, scheme.length + 1).toLowerCase() === `${scheme}:`) return "file";
  return "defer";
}

export function isSchemeLess(decision: AnchorDecision): boolean {
  return decision === "relative" || decision === "drive";
}

export function anchorKeepsHref(decision: AnchorDecision): boolean {
  return decision === "relative" || decision === "web" || decision === "mail";
}

/** A literal HTTP(S) URL prefix for browser embeds, allowing case variants. */
export function hasWebUrlPrefix(dest: string): boolean {
  return schemes.webPrefixes.some((prefix) => dest.slice(0, prefix.length).toLowerCase() === prefix);
}

/** Browser media may use web URLs and DOMPurify's data-URI subset. Local image
 * candidates still need the existing path resolver and opt-in. O(bytes), no I/O. */
export function previewResourceDecision(dest: string): "local-candidate" | "browser" | "drop" {
  const scheme = SCHEME_PREFIX.exec(dest.replace(/[\u0000-\u0020]/g, ""))?.[1]?.toLowerCase();
  if (!scheme || scheme === schemes.localFile) return "local-candidate";
  return WEB_SCHEMES.includes(scheme) || scheme === "data" ? "browser" : "drop";
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
  if (!scheme) throw new Error("unsupported url scheme");
  const nativeOnly = NATIVE_SCHEMES.includes(scheme);
  if (nativeOnly && rawScheme === scheme) return;
  if (refusedLinkScheme(scheme) || nativeOnly) {
    throw new Error(`${scheme}: links are never opened`);
  }
  if (platform === "browser") {
    new URL(dest);
    if (isWebOrMailScheme(scheme)) return;
  } else {
    if (DIRECT_OPEN_PREFIXES.some((prefix) => dest.startsWith(prefix))) return;
    if (WEB_SCHEMES.includes(scheme) && !hasWebUrlPrefix(dest)) throw new Error("unsupported web URL");
  }
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
