import schemes from "../src-tauri/external-link-schemes.json";

// Native platform.rs rechecks every OS handoff against the same refused list.
const NATIVE_SCHEMES = ["file", "tine"];
const WEB_SCHEMES = ["http", "https"];
const MAIL_SCHEME = "mailto";
// One-letter prefixes reserve a browser href but name a drive in graph content.
const SCHEME_PREFIX = /^(?:([a-z][a-z0-9+.-]+)|[a-z]):/i;

function schemePrefix(dest: string): RegExpExecArray | null {
  // Match the browser's treatment of URL whitespace, even on malformed URLs.
  return SCHEME_PREFIX.exec(dest.replace(/^[\u0000-\u0020]+|[\t\n\r]/g, ""));
}

/** The scheme left after removing every control byte and space. O(bytes). */
function compactScheme(dest: string): string | undefined {
  return SCHEME_PREFIX.exec(dest.replace(/[\u0000-\u0020]/g, ""))?.[1]?.toLowerCase();
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
  // Beta refused a scheme hidden by embedded control bytes or spaces too.
  if (!prefix) return refusedLinkScheme(compactScheme(dest) ?? "") ? "drop" : "relative";
  const scheme = prefix[1]?.toLowerCase();
  if (!scheme) return "drive";
  if (refusedLinkScheme(scheme)) return "drop";
  if (WEB_SCHEMES.includes(scheme)) return "web";
  if (scheme === MAIL_SCHEME) return "mail";
  if (scheme === "file" && dest.trim().slice(0, scheme.length + 1).toLowerCase() === `${scheme}:`) return "file";
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
  return /^https?:\/\//i.test(dest);
}

/** Browser media may use web URLs and DOMPurify's data-URI subset. Local image
 * candidates still need the existing path resolver and opt-in. O(bytes), no I/O. */
export function previewResourceDecision(dest: string): "local-candidate" | "browser" | "drop" {
  const scheme = compactScheme(dest);
  if (!scheme || scheme === "file") return "local-candidate";
  return WEB_SCHEMES.includes(scheme) || scheme === "data" ? "browser" : "drop";
}

/** Check a browser handoff (published reader, browser preview) before
 * `window.open`; relative URLs resolve against the browser's own base. Web and
 * mail open; registered schemes need exact ASCII case-insensitive consent
 * (D-20261103-S503); file, Tine and refused schemes never open. O(URL bytes). */
export function checkBrowserHandoff(dest: string, confirmedScheme: string | undefined, base: string): void {
  if (anchorDecision(dest) === "relative") {
    const here = new URL(base);
    const there = new URL(dest, here);
    if (there.protocol === here.protocol && there.host === here.host) return;
    return checkBrowserHandoff(there.href, undefined, base);
  }
  const scheme = linkScheme(dest);
  if (!scheme) throw new Error("unsupported url scheme");
  if (refusedLinkScheme(scheme) || NATIVE_SCHEMES.includes(scheme)) throw new Error(`${scheme}: links are never opened`);
  new URL(dest);
  if (isWebOrMailScheme(scheme)) return;
  const confirmed = confirmedScheme?.replace(/[A-Z]/g, (c) => c.toLowerCase());
  if (confirmed !== scheme) throw new Error(`opening a ${scheme}: link needs your confirmation`);
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
