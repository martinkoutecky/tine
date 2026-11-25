import schemes from "../src-tauri/external-link-schemes.json";
import { link_anchor_route, link_preview_route, link_scheme, link_scheme_prefix_bytes } from "./render/wasm/lsdoc_wasm.js";

// Which scheme route a destination takes is one rule in core
// (crates/tine-core/src/link_scheme.rs), shared with the link-target owner;
// this module only names its answers. Native platform.rs rechecks every OS
// handoff against the same refused list.
const NATIVE_SCHEMES = ["file", "tine"];

// Core's rule reads only a bounded prefix, so only that crosses into wasm. The
// bound is in bytes; a slice of that many UTF-16 units is at least as long.
let prefixBytes: number | undefined;
function head(dest: string): string {
  prefixBytes ??= link_scheme_prefix_bytes();
  return dest.length > prefixBytes ? dest.slice(0, prefixBytes) : dest;
}

/** The lowercased URL scheme for consent, reserving single letters for drives. O(bytes). */
export function linkScheme(dest: string): string | null {
  return link_scheme(head(dest)) ?? null;
}

export function refusedLinkScheme(scheme: string): boolean {
  return schemes.refused.includes(scheme);
}

/** Saved consent is for valid registered app scheme names. */
export function rememberableLinkScheme(scheme: string): boolean {
  // A valid name's own route is "defer" unless it is web, mail, file: or refused.
  return /^[a-z][a-z0-9+.-]+$/.test(scheme) && anchorDecision(`${scheme}:`) === "defer"
    && !NATIVE_SCHEMES.includes(scheme);
}

/** Classify an authored destination before attachment, file or media routing.
 * Relative, web and mail links keep hrefs. Drive prefixes enter the scheme-less
 * routes, with browser navigation suppressed. Custom schemes defer to consent.
 * O(destination bytes), with no I/O. */
export type AnchorDecision = "relative" | "web" | "mail" | "file" | "drive" | "defer" | "drop";
const ROUTES: readonly AnchorDecision[] = ["relative", "web", "mail", "file", "drive", "defer", "drop"];
const PREVIEWS = ["local-candidate", "browser", "drop"] as const;
export function anchorDecision(dest: string): AnchorDecision {
  return ROUTES[link_anchor_route(head(dest))];
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
  return PREVIEWS[link_preview_route(head(dest))];
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
  const decision = anchorDecision(dest);
  if (decision === "web" || decision === "mail") return;
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
