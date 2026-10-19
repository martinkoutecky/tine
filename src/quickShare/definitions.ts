// A quick share in the graph (D-20261017-QS3, D-20261018-Q1, D-20261018-QS3; docs/contracts/export-definitions.md):
// the share's export definition is a query block carrying `tine.share:: <login>/<gist_id>`. The key never enters
// the graph (it stays in this device's `quick-shares.json`), so the property names the gist and nothing that opens
// it. From share start until GitHub answers, the same block carries `tine.share:: pending/<record id>`, so a share
// whose outcome is confirmed later (Settings → Sharing → Check) still finds its block.
// Every write here is an ordinary block edit through the document layer (setBlockProperty, appendQueryBlock,
// deleteBlock) and its normal guarded save; this module adds no write path.
import { backend } from "../backend";
import {
  appendQueryBlock, blockProperty, blockWritable, childIds, deleteBlock, ensurePageLoaded, flushPage, node, pageByName,
  pageRoots, reportPageLoadRefusal, setBlockProperty,
} from "../document";
import { SHARE_PROPERTY, ensureExportsPage } from "../exportsPage";
import { queryMacroExtents } from "../editor/queryMacro";
import { bindingOwner, ownedWhen, readOwned, type Owner, type WriteOwner } from "../owned";
import type { PageKind } from "../types";

/** The property value of a share: `<login>/<gist_id>`, never the key. */
export function shareValue(login: string, gistId: string): string {
  return `${login}/${gistId}`;
}

/** The gist a `tine.share::` value names, whatever login prefix it carries (an account rename keeps the gist id;
 *  GitHub gist ids are unique across accounts). */
export function shareGistId(value: string): string {
  return value.slice(value.lastIndexOf("/") + 1);
}

/** A share in flight (D-20261018-QS3): `tine.share:: pending/<record id>`, the share id of this device's record
 *  (`quick-shares.json`; not secret, it is in the envelope header). Success replaces it with `<login>/<gist_id>`,
 *  a definite failure removes it, an unknown outcome keeps it. Record ids are drawn so that their spelling never
 *  has a gist id's shape (`fresh_share_id` in quick_share.rs), so a share of a GitHub login `pending` stays a share. */
export const PENDING_PREFIX = "pending/";
export function pendingValue(shareId: string): string {
  return `${PENDING_PREFIX}${shareId}`;
}
const RECORD_ID = /^[A-Za-z0-9_-]{22}$/;
const GIST_ID = /^[0-9a-f]{20,32}$/;
/** The record id a pending marker names, or null for a share (`<login>/<gist_id>`) or anything else. */
export function pendingRecordId(value: string): string | null {
  if (!value.startsWith(PENDING_PREFIX)) return null;
  const id = value.slice(PENDING_PREFIX.length);
  return RECORD_ID.test(id) && !GIST_ID.test(id) ? id : null;
}

/** Thrown when the graph the bookkeeping started on is no longer the bound graph: the write is cancelled. */
export const GRAPH_CHANGED = "graph-changed";

/** Whether the block still carries the query the share was made from (the dialog may have changed it). Compared
 *  as the share dialog builds it: `{{<name> <trimmed argument>}}` (Macro.tsx). */
function carriesQuery(blockId: string, macro: string): boolean {
  const raw = node(blockId)?.raw ?? "";
  const wanted = macro.trim();
  return queryMacroExtents(raw).some((extent) => `{{${extent.name} ${extent.argument.trim()}}}` === wanted);
}

/** Where the share dialog started: the query block it opened on (when its query was not changed), and the query
 *  text a new definition carries. */
export interface ShareDefinitionTarget {
  hostBlockId: string | null;
  macro: string;
}

/** Mark a share as started, before its upload (D-20261018-QS3): `tine.share:: pending/<shareId>` goes on the query
 *  block it started from, when that block can be edited, carries the query the share is made from, and does not
 *  already carry a `tine.share::` (another share, or another share still pending: re-sharing is phase 3, and
 *  overwriting would drop the earlier share from the Exports page while its gist lives on); otherwise on a new
 *  definition (the query plus the marker) appended to the exports page. Saved before it answers: the caller starts
 *  the upload only then. When the marker did not reach disk it is taken back out of the working set (the marker
 *  off the block, an appended definition deleted), so a later save never writes a marker no upload stands behind,
 *  and this throws with a reason the dialog shows. `owner` is the graph binding the share starts on: rechecked
 *  before every edit and after every await, so a graph switch throws GRAPH_CHANGED instead of editing the
 *  replacement graph. Answers the page that holds the marker. */
export async function markSharePending(target: ShareDefinitionTarget, shareId: string, owner: WriteOwner = bindingOwner()): Promise<string> {
  const value = pendingValue(shareId);
  const bound = () => { if (!owner()) throw new Error(GRAPH_CHANGED); };
  bound();
  const host = target.hostBlockId;
  if (host && blockWritable(host) && carriesQuery(host, target.macro) && blockProperty(host, SHARE_PROPERTY) === null) {
    setBlockProperty(host, SHARE_PROPERTY, value);
    const page = node(host)?.page;
    if (blockProperty(host, SHARE_PROPERTY) !== value || !page) throw new Error("the query block can't be edited");
    const saved = await flushPage(page);
    bound();
    if (!saved) {
      if (blockProperty(host, SHARE_PROPERTY) === value) setBlockProperty(host, SHARE_PROPERTY, null);
      throw new Error(`couldn't save ${page}`);
    }
    return page;
  }
  const page = await ensureExportsPage();
  bound();
  const appended = await appendQueryBlock(page, target.macro, [[SHARE_PROPERTY, value]]);
  bound();
  if (!appended) {
    // The record id is fresh, so a block carrying it on this page is the one this call appended.
    if (pageByName(page)) for (const id of pageBlocks(page)) if (blockProperty(id, SHARE_PROPERTY) === value) deleteBlock(id);
    throw new Error(`couldn't write to ${page}`);
  }
  return page;
}

/** The blocks of a loaded page, depth first. */
function pageBlocks(page: string): string[] {
  const out: string[] = [];
  const walk = (ids: readonly string[]) => { for (const id of ids) { out.push(id); walk(childIds(id)); } };
  walk(pageRoots(page));
  return out;
}

/** Remove every `tine.share::` naming `gistId` (a share was deleted or forgotten), wherever the definition lives
 *  and whatever login prefix it carries (an account rename). See rewriteShareProperty for how pages are found,
 *  loaded and saved. Answers the pages it changed; throws when a page holding the share could not be changed, so
 *  the caller says so instead of leaving a stale listing unmentioned. */
export async function removeShareProperty(gistId: string, bound: WriteOwner = bindingOwner()): Promise<string[]> {
  if (!gistId) return [];
  return rewriteShareProperty(gistId, (value) => pendingRecordId(value) === null && shareGistId(value) === gistId, null, bound);
}

/** Finish a share's pending marker (D-20261018-QS3): every `tine.share:: pending/<shareId>` becomes `next`
 *  (`<login>/<gist_id>` once the gist exists) or goes (`null`: the share was not created, or was forgotten). Found
 *  by the property wherever the block is now (edited or moved since the start), on every block carrying it (a copied
 *  block, a synced duplicate: each is a definition of this share). `pages` names pages to look at even when the
 *  engine's index does not list them yet (the page the marker was just saved to). Answers the pages it changed:
 *  none when no block carries the marker (its block was deleted, or the marker removed), which the caller says. */
export async function finishSharePending(shareId: string, next: string | null, bound: WriteOwner = bindingOwner(), pages: readonly string[] = []): Promise<string[]> {
  if (!shareId) return [];
  const marker = pendingValue(shareId);
  return rewriteShareProperty(marker, (value) => value === marker, next, bound, pages);
}

/** One pending marker in the graph: its record id and the pages whose blocks carry it. */
export interface PendingShareMarker { shareId: string; pages: string[] }
export interface PendingShareMarkers { markers: PendingShareMarker[]; exceeded: boolean }

/** Every pending marker in the bound graph (Settings → Sharing), found through the same `(property tine.share)`
 *  query the Exports page lists definitions with, reading each block's parsed properties. `exceeded`: the answer
 *  was cut, so some markers may be missing. Null when the graph changed meanwhile. */
export async function pendingShareMarkers(owner: Owner = bindingOwner()): Promise<PendingShareMarkers | null> {
  const answer = await sharePropertyQuery(owner);
  if (!answer) return null;
  const byId = new Map<string, Set<string>>();
  for (const group of answer.groups) {
    for (const block of group.blocks) {
      for (const [key, value] of block.properties ?? []) {
        const id = key.toLowerCase() === SHARE_PROPERTY ? pendingRecordId(value.trim()) : null;
        if (!id) continue;
        const pages = byId.get(id) ?? new Set<string>();
        pages.add(group.page);
        byId.set(id, pages);
      }
    }
  }
  return { markers: [...byId].map(([shareId, pages]) => ({ shareId, pages: [...pages].sort() })), exceeded: answer.exceeded };
}

async function sharePropertyQuery(bound: Owner) {
  const owner = ownedWhen(bound);
  if (!owner()) return null;
  const parsed = await readOwned(owner, backend().parseQuery(`(property ${SHARE_PROPERTY})`, "og"));
  if (parsed.kind === "stale") return null;
  const result = await readOwned(owner, backend().queryRun(parsed.value.query, parsed.value.view));
  if (result.kind === "stale" || result.value.anchor !== "block") return null;
  return result.value;
}

/** Set every `tine.share::` whose value `matches` to `next` (null removes it). The engine's `(property tine.share)`
 *  finds the pages (those whose matching raw holds `needle`), plus the `extra` pages; each is loaded into the working
 *  set (a read; a loaded page keeps its unsaved edits) and the property goes through setBlockProperty and the
 *  normal save. `bound` is the graph binding the action started on: rechecked after every await and before every
 *  edit, so a graph switch stops the rewrite before it touches the replacement graph. */
async function rewriteShareProperty(needle: string, matches: (value: string) => boolean, next: string | null, bound: WriteOwner, extra: readonly string[] = []): Promise<string[]> {
  const owner = ownedWhen(bound);
  const carries = (id: string) => { const value = blockProperty(id, SHARE_PROPERTY); return value !== null && matches(value); };
  const answer = await sharePropertyQuery(owner);
  if (!answer) return [];
  const pages = new Map<string, PageKind>();
  for (const group of answer.groups) if (group.blocks.some((block) => block.raw.includes(needle))) pages.set(group.page, group.kind);
  for (const name of extra) if (!pages.has(name)) pages.set(name, pageByName(name)?.kind ?? "page");
  const changed: string[] = [];
  const failed: string[] = [];
  for (const [name, kind] of pages) {
    if (!owner()) return changed;
    if (!pageByName(name)) {
      const loaded = await readOwned(owner, backend().getPage(name, kind));
      if (loaded.kind === "stale") return changed;
      if (!loaded.value) continue;
      const refusal = ensurePageLoaded(loaded.value);
      if (refusal) { reportPageLoadRefusal(refusal, "Its share property was not changed."); failed.push(name); continue; }
    }
    const ids = pageBlocks(name).filter(carries);
    if (!ids.length) continue;
    for (const id of ids) setBlockProperty(id, SHARE_PROPERTY, next);
    if (ids.some(carries)) { failed.push(name); continue; }
    const saved = await flushPage(name);
    if (!owner()) return changed;
    if (saved) changed.push(name); else failed.push(name);
  }
  if (failed.length) {
    throw new Error(next === null ? `couldn't remove ${SHARE_PROPERTY}:: …/${needle} from ${failed.join(", ")}`
      : `couldn't write ${SHARE_PROPERTY}:: ${next} on ${failed.join(", ")}`);
  }
  if (answer.exceeded) throw new Error(`more blocks carry ${SHARE_PROPERTY} than one query answers; check the Exports page`);
  return changed;
}
