import type { PaneRouter, Route } from "./router";
import type { PublishedSnapshot } from "./publishedBackend";
import type { BlockDto, PageDto } from "./types";
import { pageIdentityKey } from "./pageIdentity";
import { blockRegions } from "./render/parse";

export type PublishedPermalinkTarget =
  | { kind: "page"; page: string }
  | { kind: "block"; block: string };

export interface ResolvedPublishedPermalink {
  page: PublishedSnapshot["pages"][number];
  block?: string;
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

/** Absolute copyable URL, preserving the deployment path and query string. */
export function publishedPermalinkUrl(
  target: PublishedPermalinkTarget,
  currentHref: string = window.location.href,
): string {
  const url = new URL(currentHref);
  url.hash = publishedPermalinkHash(target).slice(1);
  return url.href;
}

function findBlock(blocks: PageDto["blocks"], wanted: string, format: "md" | "org"): BlockDto | null {
  for (const block of blocks) {
    if ((blockRegions(block.raw, format).id?.value.trim() ?? block.id) === wanted) return block;
    const child = findBlock(block.children, wanted, format);
    if (child) return child;
  }
  return null;
}

/** Resolve a link only within the baked snapshot. Page aliases remain valid,
 *  while block UUIDs are graph-wide so moving a block does not break its link
 *  after the next export. Canonical page identity and parser-owned block IDs
 *  match publishedBackend. O(snapshot blocks) for a block, O(pages + aliases)
 *  for a page. Returns null for an absent target; parser refusal is observable. */
export function resolvePublishedPermalink(
  snapshot: PublishedSnapshot,
  target: PublishedPermalinkTarget,
): ResolvedPublishedPermalink | null {
  if (target.kind === "block") {
    for (const page of snapshot.pages) {
      if (findBlock(page.blocks, target.block, page.format ?? "md")) return { page, block: target.block };
    }
    return null;
  }

  const wanted = pageIdentityKey(target.page);
  let page = snapshot.pages.find((candidate) => pageIdentityKey(candidate.name) === wanted);
  if (!page) {
    const alias = snapshot.aliases.find(([from]) => pageIdentityKey(from) === wanted);
    if (alias) {
      page = snapshot.pages.find((candidate) => pageIdentityKey(candidate.name) === pageIdentityKey(alias[1]));
    }
  }
  return page ? { page } : null;
}

export type OpenPublishedPermalinkResult =
  | { status: "ignored" | "invalid" | "missing" }
  | { status: "opened"; target: PublishedPermalinkTarget; route: Route };

/** Apply a public fragment to one pane after the snapshot is ready. */
export function openPublishedPermalink(
  snapshot: PublishedSnapshot,
  hash: string,
  router: PaneRouter,
): OpenPublishedPermalinkResult {
  if (!hash) return { status: "ignored" };
  const target = parsePublishedPermalinkHash(hash);
  if (!target) return { status: "invalid" };
  const resolved = resolvePublishedPermalink(snapshot, target);
  if (!resolved) return { status: "missing" };
  const pageTarget = {
    name: resolved.page.name,
    pageKind: resolved.page.kind,
    ...(resolved.page.path ? { path: resolved.page.path } : {}),
  };
  if (resolved.block) router.openPageAtBlock({ ...pageTarget, block: resolved.block });
  else router.openPageTarget(pageTarget);
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
  const nextHash = target ? publishedPermalinkHash(target) : "";
  if (browser.location.hash === nextHash) return;
  const url = new URL(browser.location.href);
  url.hash = nextHash ? nextHash.slice(1) : "";
  browser.history.replaceState(browser.history.state, "", url.href);
}
