import schemes from "../src-tauri/external-link-schemes.json";

const NATIVE_SCHEMES = [schemes.localFile, schemes.deepLink];

/** Absolute URL scheme as interpreted by the browser. A single letter is
 * reserved for Windows drives. O(bytes). */
export function linkScheme(dest: string): string | null {
  let protocol: string | undefined;
  try {
    protocol = new URL(dest).protocol;
  } catch (error) {
    if (!(error instanceof TypeError)) throw error;
  }
  return protocol && protocol.length > 2 ? protocol.slice(0, -1) : null;
}

export function refusedLinkScheme(scheme: string): boolean {
  return schemes.refused.includes(scheme);
}

/** Whether a parsed scheme can go directly to a browser or OS URL opener. */
export function isDirectExternalScheme(scheme: string): boolean {
  return schemes.web.includes(scheme) || scheme === schemes.mail;
}

/** Only exact, valid scheme names eligible for the existing prompt can be saved.
 * No host, path, prefix, direct-open or refused scheme can become consent. */
export function rememberableLinkScheme(scheme: string): boolean {
  return !isDirectExternalScheme(scheme) && !NATIVE_SCHEMES.includes(scheme)
    && !refusedLinkScheme(scheme) && /^[a-z][a-z0-9+.-]+$/.test(scheme);
}

/** Check an outbound URL before handing it to the browser or native command.
 * A custom scheme needs confirmation for that exact scheme (D-20261103-S503).
 * Local files and Tine links have dedicated native routes. Throws on refusal.
 * Cost is O(URL bytes), with no I/O. */
export function checkExternalLink(
  dest: string,
  confirmedScheme: string | undefined,
  platform: "browser" | "native",
): void {
  const scheme = linkScheme(dest);
  if (!scheme) throw new Error("unsupported url scheme");
  const nativeOnly = NATIVE_SCHEMES.includes(scheme);
  if (refusedLinkScheme(scheme) || (platform === "browser" && nativeOnly)) {
    throw new Error(`${scheme}: links are never opened`);
  }
  if (!isDirectExternalScheme(scheme) && !nativeOnly && !sameScheme(confirmedScheme, scheme)) {
    throw new Error(`opening a ${scheme}: link needs your confirmation`);
  }
}

/** The browser backends' handoff check (published reader, browser preview).
 * A scheme-less link that resolves to `base`'s own site (`#section`,
 * `other.html`, `*Heading`) opens as on the Beta, a new tab; `//host`, `\\host`
 * and `/\host` leave the site and, like every other destination, go through
 * `checkExternalLink`. */
export function checkBrowserHandoff(url: string, confirmedScheme: string | undefined, base: string): void {
  if (anchorDecision(url) === "relative") {
    try {
      const here = new URL(base);
      const there = new URL(url, here);
      if (there.protocol === here.protocol && there.host === here.host) return;
    } catch (error) {
      if (!(error instanceof TypeError)) throw error;
    }
  }
  checkExternalLink(url, confirmedScheme, "browser");
}

/** Exact ASCII case-insensitive match, as the native opener's
 * `eq_ignore_ascii_case` (no Unicode folding: KELVIN SIGN is not `k`). */
function sameScheme(confirmed: string | undefined, scheme: string): boolean {
  return confirmed !== undefined && confirmed.length === scheme.length
    && confirmed.replace(/[A-Z]/g, (c) => c.toLowerCase()) === scheme;
}

/** What an anchor in graph content does with its destination, in every renderer
 * and mode (GH #984). `relative`: site- or page-internal, a real href. `direct`:
 * web or mail, a real href. `defer`: any other scheme (another app's, `file:`,
 * `tine:`, a one-letter one): no href, so no browser default navigation, middle-
 * click or new tab; it opens only through the checked opener. `drop`: a refused
 * scheme, never opened. One URL parse, O(URL bytes). */
export type AnchorDecision = "relative" | "direct" | "defer" | "drop";
export function anchorDecision(dest: string): AnchorDecision {
  let scheme: string;
  try {
    scheme = new URL(dest).protocol.slice(0, -1);
  } catch (error) {
    if (!(error instanceof TypeError)) throw error;
    return "relative";
  }
  if (refusedLinkScheme(scheme)) return "drop";
  return isDirectExternalScheme(scheme) ? "direct" : "defer";
}

/** Where a raw-HTML anchor keeps a deferred destination (GH #984). */
export const DEFERRED_HREF = "data-tine-href";

/** Apply `anchorDecision` to a sanitized anchor: keep the href, move it to
 * DEFERRED_HREF (still a focusable link), or drop a refused one. */
export function deferAnchorHref(anchor: Element): void {
  const decision = anchorDecision(anchor.getAttribute("href") ?? "");
  if (decision === "relative" || decision === "direct") return;
  const dest = anchor.getAttribute("href")!;
  anchor.removeAttribute("href");
  if (decision === "drop") return;
  anchor.setAttribute(DEFERRED_HREF, dest);
  anchor.setAttribute("role", "link");
  anchor.setAttribute("tabindex", "0");
}
