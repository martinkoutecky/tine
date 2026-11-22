import type { PaneRouter, Route } from "./router";
import type { Backend } from "./backend";
import type { linkTarget } from "./pageIndex";
import { ownerPath } from "./inventoryRows";

export type PublishedPermalinkTarget =
  | { kind: "page"; page: string }
  | { kind: "block"; block: string };

/** In a quick share viewer, the key is the part of the fragment
 *  the reader never changes. Permalinks are written and read after it. */
let sharePrefix: string | null = null;

/** Set by the snapshot loader once a share opened; null outside share mode. */
export function setQuickSharePrefix(prefix: string | null): void {
  sharePrefix = prefix;
}

/** The fragment as the public permalink grammar sees it: a share's own
 *  prefix removed, so `#<prefix>/page/x` reads as `#/page/x`. */
function permalinkPart(hash: string): string {
  if (sharePrefix === null) return hash;
  const own = `#${sharePrefix}`;
  return hash === own ? "" : hash.startsWith(`${own}/`) ? `#${hash.slice(own.length)}` : "";
}

const PAGE_PREFIX = "#/page/";
const BLOCK_PREFIX = "#/block/";

function decodeSegment(value: string): string | null {
  try {
    const decoded = decodeURIComponent(value);
    return decoded.length > 0 ? decoded : null;
  } catch {
    return null;
  }
}

/** Stable, static-host-safe fragment for a published page or block. */
export function publishedPermalinkHash(target: PublishedPermalinkTarget): string {
  return target.kind === "page"
    ? `${PAGE_PREFIX}${encodeURIComponent(target.page)}`
    : `${BLOCK_PREFIX}${encodeURIComponent(target.block)}`;
}

/** Parse only the public permalink grammar. */
export function parsePublishedPermalinkHash(hash: string): PublishedPermalinkTarget | null {
  const match = hash.startsWith(PAGE_PREFIX)
    ? { kind: "page" as const, encoded: hash.slice(PAGE_PREFIX.length) }
    : hash.startsWith(BLOCK_PREFIX)
      ? { kind: "block" as const, encoded: hash.slice(BLOCK_PREFIX.length) }
      : null;
  if (!match || match.encoded.includes("/")) return null;
  const decoded = decodeSegment(match.encoded);
  if (!decoded) return null;
  return match.kind === "page"
    ? { kind: "page", page: decoded }
    : { kind: "block", block: decoded };
}

export type OpenPublishedPermalinkResult =
  | { status: "ignored" | "invalid" | "missing" }
  | { status: "opened"; target: PublishedPermalinkTarget; route: Route };

/** Apply a public fragment after graph open. Page links use the core's
 *  inventory routing, and block links use its indexed UUID resolution. */
export async function openPublishedPermalink(
  hash: string,
  router: PaneRouter,
  reads: Pick<Backend, "resolvePage" | "resolveBlock">,
  pageLink: typeof linkTarget,
  current: () => boolean,
): Promise<OpenPublishedPermalinkResult> {
  hash = permalinkPart(hash);
  if (!hash) return { status: "ignored" };
  const target = parsePublishedPermalinkHash(hash);
  if (!target) return { status: "invalid" };
  if (target.kind === "block") {
    const group = await reads.resolveBlock(target.block);
    if (!current()) return { status: "ignored" };
    if (!group) return { status: "missing" };
    router.openPageAtBlock({ name: group.page, pageKind: group.kind, path: group.path, block: target.block });
  } else {
    const link = pageLink(target.page);
    const path = ownerPath(await reads.resolvePage(link.name, link.kind));
    if (!current()) return { status: "ignored" };
    if (!path) return { status: "missing" };
    router.openPageTarget({ name: link.name, pageKind: link.kind, path });
  }
  return { status: "opened", target, route: router.route() };
}

/** `undefined` means the workspace is ambiguous and the existing address must
 *  stay untouched. `null` means it is unambiguous but is not showing a page. */
export function publishedPermalinkForWorkspace(
  paneCount: number,
  tabCount: number,
  route: Route,
  revealedBlock?: string,
): PublishedPermalinkTarget | null | undefined {
  if (paneCount !== 1 || tabCount !== 1) return undefined;
  if (route.kind !== "page") return null;
  const block = route.block ?? revealedBlock;
  return block ? { kind: "block", block } : { kind: "page", page: route.name };
}

/** Replace the displayed fragment without adding a browser-history entry. */
export function replacePublishedPermalink(
  target: PublishedPermalinkTarget | null,
  browser: Pick<Window, "location" | "history"> = window,
): void {
  const permalink = target ? publishedPermalinkHash(target) : "";
  const nextHash = sharePrefix === null ? permalink : `#${sharePrefix}${permalink.slice(1)}`;
  if (browser.location.hash === nextHash) return;
  const url = new URL(browser.location.href);
  url.hash = nextHash ? nextHash.slice(1) : "";
  browser.history.replaceState(browser.history.state, "", url.href);
}
