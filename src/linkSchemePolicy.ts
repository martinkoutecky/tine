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
    && !refusedLinkScheme(scheme) && linkScheme(`${scheme}:`) === scheme;
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

/** Exact ASCII case-insensitive match, as the native opener's
 * `eq_ignore_ascii_case` (no Unicode folding: KELVIN SIGN is not `k`). */
function sameScheme(confirmed: string | undefined, scheme: string): boolean {
  return confirmed !== undefined && confirmed.length === scheme.length
    && confirmed.replace(/[A-Z]/g, (c) => c.toLowerCase()) === scheme;
}

/** Whether an anchor may keep `dest` as its `href`, i.e. reach browser default
 * navigation (middle-click, "open in new tab"). Relative destinations do (site-
 * internal); refused schemes never do; in a published reader only web and mail
 * do (GH #984), so every other scheme, one-letter ones included, is opened only
 * through the checked opener. O(URL bytes). */
export function navigableHref(dest: string, published: boolean): boolean {
  let scheme: string;
  try {
    scheme = new URL(dest).protocol.slice(0, -1);
  } catch (error) {
    if (!(error instanceof TypeError)) throw error;
    return true;
  }
  if (refusedLinkScheme(scheme)) return false;
  return !published || isDirectExternalScheme(scheme);
}

/** Where a raw-HTML anchor keeps a destination that must not reach browser
 * navigation but still opens through the checked opener (GH #984). */
export const DEFERRED_HREF = "data-tine-href";

/** Apply `navigableHref` to a sanitized anchor: keep the href, or move it to
 * DEFERRED_HREF (still a focusable link), or drop a refused one. O(URL bytes). */
export function deferAnchorHref(anchor: Element, published: boolean): void {
  const dest = anchor.getAttribute("href") ?? "";
  if (navigableHref(dest, published)) return;
  anchor.removeAttribute("href");
  if (navigableHref(dest, false)) {
    anchor.setAttribute(DEFERRED_HREF, dest);
    anchor.setAttribute("role", "link");
    anchor.setAttribute("tabindex", "0");
  }
}
