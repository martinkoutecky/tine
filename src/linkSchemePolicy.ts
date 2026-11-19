import schemes from "../src-tauri/external-link-schemes.json";
import { asset_rel_path, pdf_asset_file } from "./render/wasm/lsdoc_wasm.js";
import { filePathUrl, graphRelative, type FileLinkTarget } from "./render/relativeFileLink";

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
  if (!authoredScheme(url)) {
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

function authoredScheme(dest: string): string | undefined {
  if (!dest.includes(":")) return undefined;
  // Apply the URL parser's whitespace rules before reading the scheme. Even
  // a malformed absolute URL has a scheme and cannot become a relative path.
  return /^[\u0000-\u0020]*([a-z][a-z0-9+.-]*):/i.exec(dest.replace(/[\t\n\r]/g, ""))?.[1].toLowerCase();
}

export type AnchorDecision =
  | { kind: "relative"; url: string; target: FileLinkTarget | null; pathQualified: boolean; useUrl: boolean;
      attachment: string | null; mediaPath: string | null; isPdf: boolean; pdfAsset: string | null }
  | { kind: "file"; url: string; target: FileLinkTarget | null; useUrl: boolean }
  | { kind: "drive"; target: { kind: "outside"; reason: "absolute"; path: string } }
  | { kind: "web"; url: string }
  | { kind: "mail" | "defer"; url: string }
  | { kind: "drop"; error: string };

function decodePath(path: string): string | null {
  if (!path.includes("%")) return path;
  try { return decodeURIComponent(path); }
  catch (error) {
    if (!(error instanceof URIError)) throw error;
    return null;
  }
}

/** Classify a graph-authored destination (GH #984). One decoded, normalized
 * path supplies all local routing. Literal URL syntax and its escaped
 * payload are preserved for URL handoffs. Cost O(destination bytes), no I/O. */
export function anchorDecision(dest: string): AnchorDecision {
  const trimmed = dest.trim();
  const decoded = decodePath(trimmed);
  let path = decoded ?? trimmed;
  if (path.includes("\\")) path = path.replaceAll("\\", "/");
  // Logseq's protocol-url? requires a literal scheme delimiter. Encoded
  // colons in ordinary filenames stay path bytes, e.g. report%3Aold.txt.
  const scheme = authoredScheme(dest);
  if (scheme && refusedLinkScheme(scheme)) return { kind: "drop", error: `${scheme}: links are never opened` };
  if (scheme && isDirectExternalScheme(scheme)) return { kind: scheme === schemes.mail ? "mail" : "web", url: dest };
  const file = scheme === schemes.localFile;
  if (file) {
    if (!/^file:(?![/\\])/i.test(trimmed)) return { kind: "file", url: dest, target: null, useUrl: true };
    path = path.slice("file:".length);
  } else if (scheme && scheme.length > 1) {
    return { kind: "defer", url: dest };
  }

  const drive = path[1] === ":" ? /^[a-z]:(\/)?/i.exec(path) : null;
  if (!file && (drive?.[1] || scheme?.length === 1)) return drive?.[1] && decoded !== null && !path.includes("\0")
    ? { kind: "drive", target: { kind: "outside", reason: "absolute", path } }
    : { kind: "drop", error: "unsupported url scheme" };

  let attachment: string | null = null;
  let mediaPath: string | null = null;
  const hasSeparator = path.includes("/");
  if (!file && (hasSeparator || path.length === "assets".length) && /assets/i.test(path)) {
    // Attachment lookup strips trailing separators. Only an exact assets
    // segment can name the assets root (GH #367).
    const folder = path.endsWith("/") ? path.replace(/\/+$/, "") : path;
    attachment = /(^|\/)assets$/i.test(folder) ? "" : asset_rel_path(folder) ?? null;
    if (attachment !== null) mediaPath = folder === dest && attachment !== "" ? attachment : asset_rel_path(dest) ?? null;
  }
  // The outside PDF route requires authored path syntax (GH #732). A URL
  // escape inside a bare asset name never opts it into the outside reader.
  const pathQualified = hasSeparator && /[/\\]/.test(trimmed);
  const dotted = path.includes(".");
  const isPdf = dotted && /\.pdf$/i.test(path);
  const useUrl = isPdf && !pathQualified;
  let target: FileLinkTarget | null = null;
  if (attachment === null && !/^[#*]/.test(path)) {
    if (drive?.[1] || path.startsWith("/")) target = { kind: "outside", reason: "absolute", path };
    else if (dotted) target = graphRelative(path);
  }
  if (file) return { kind: "file", url: dest, target, useUrl };
  // The shared PDF identity API accepts escaped input. Protect literal percent
  // bytes so it sees the same path, including names containing an escape.
  const pdfAsset = isPdf && target?.kind !== "outside" ? pdf_asset_file(path.replaceAll("%", "%25")) : null;
  return { kind: "relative", url: dest, target: useUrl && pdfAsset !== null ? null : target,
    pathQualified, useUrl, attachment, mediaPath, isPdf, pdfAsset };
}

export function anchorHref(decision: AnchorDecision): string | undefined {
  if (decision.kind === "relative" || decision.kind === "web" || decision.kind === "mail") return decision.url;
}

/** Where a raw-HTML anchor keeps a deferred destination (GH #984). */
export const DEFERRED_HREF = "data-tine-href";

/** Apply `anchorDecision` to a sanitized anchor: keep the href, move it to
 * DEFERRED_HREF (still a focusable link), or drop a refused one. */
export function deferAnchorHref(anchor: Element): void {
  const decision = anchorDecision(anchor.getAttribute("href") ?? "");
  if (anchorHref(decision) !== undefined) return;
  anchor.removeAttribute("href");
  if (decision.kind === "drop") return;
  anchor.setAttribute(DEFERRED_HREF, decision.kind === "drive" ? filePathUrl(decision.target.path) : decision.url);
  anchor.setAttribute("role", "link");
  anchor.setAttribute("tabindex", "0");
}
