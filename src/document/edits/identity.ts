import { type Format, type PageKind } from "../../types";
import { doc, formatForBlock, pageByName, setDoc } from "../model";
import { captureBinding, bindingCurrent } from "../../binding";
import { readOwned, bindingOwner } from "../../owned";
import { blockWritable } from "./properties";
import { markDirty, flushPage, isConflicted, persistTogether } from "../save/engine";
import { backend } from "../../backend";
import { ensurePageLoaded } from "../workingSet";
import { editBlock, type BlockIdentityFacts } from "../../render/parse";

export { existingBlockId } from "../../blockIdentity";
import { existingBlockId } from "../../blockIdentity";

/** The identity other blocks and persisted UI state must use for a loaded node.
 * A freshly-created node keeps its transient `b…` store key for the whole live
 * session even after Copy block ref writes a UUID property into `raw`; external
 * references must follow that property while render/edit paths keep the key. */
export function blockExternalId(id: string, facts?: BlockIdentityFacts): string | null {
  const node = doc.byId[id];
  if (!node) return null;
  return existingBlockId(node.raw, formatForBlock(id), facts) ?? node.id;
}

export interface LoadedBlockRef {
  uuid: string;
  page: string;
  pageKind: PageKind;
  path?: string;
  /** Sibling-index path from the page's top level to an ID-less block. Written
   * only into a saved session (see {@link blockPositionRef}) and consumed once,
   * on the first resolution after a restart ({@link settleBlockRef}); `uuid`
   * is then a stale locator that must not be trusted. */
  blockPos?: readonly number[];
}

/** The block at `pos` (sibling indices from the page's top level), or null. */
function blockAtPosition(roots: readonly string[], pos: readonly number[]): string | null {
  let siblings = roots;
  let id: string | null = null;
  for (const index of pos) {
    id = siblings[index] ?? null;
    if (id === null) return null;
    siblings = doc.byId[id]?.children ?? [];
  }
  return id;
}

/** The sibling-index path of `target` under `roots`, or null. O(page blocks). */
function positionOf(roots: readonly string[], target: string): number[] | null {
  const walk = (ids: readonly string[], prefix: number[]): number[] | null => {
    for (let i = 0; i < ids.length; i++) {
      if (ids[i] === target) return [...prefix, i];
      const found = walk(doc.byId[ids[i]]?.children ?? [], [...prefix, i]);
      if (found) return found;
    }
    return null;
  };
  return walk(roots, []);
}

/** Resolve a durable external UUID back to the current live store key. The page
 * descriptor is part of the identity. A unique authored `id` claimant wins; two
 * authored claimants are ambiguous and resolve to null. A runtime store key is
 * only a fallback locator for a block with no authored id, so a runtime key never
 * acts as a second identity (GH #373). `navigation` is for a route or sidebar
 * item that opened an ID-less block by its runtime key and must keep showing it
 * after a reference to it stamps an `id::` in the same session: there the key
 * stays a locator for a block on the ref's page even once it has an id. */
export function resolveBlockRef(ref: LoadedBlockRef, opts: { navigation?: boolean } = {}): string | null {
  const owner = pageByName(ref.page);
  if (
    !owner
    || owner.kind !== ref.pageKind
    || (ref.path !== undefined && owner.id !== ref.path)
  ) return null;

  if (ref.blockPos) {
    // A restored, not-yet-settled ref: only the position names the block, and
    // only an ID-less block can have been saved by position.
    const id = blockAtPosition(owner.roots, ref.blockPos);
    const node = id ? doc.byId[id] : undefined;
    return id && node && node.page === ref.page && existingBlockId(node.raw, formatForBlock(id)) === null ? id : null;
  }

  const stack = [...owner.roots];
  const seen = new Set<string>();
  let authoredClaim: string | null = null;
  while (stack.length) {
    const id = stack.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    const node = doc.byId[id];
    if (!node || node.page !== ref.page) continue;
    if (existingBlockId(node.raw, formatForBlock(id)) === ref.uuid) {
      // A second authored claimant is ambiguous. Never guess, and never rewrite
      // either block merely because a route exposed the conflict.
      if (authoredClaim !== null) return null;
      authoredClaim = id;
    }
    stack.push(...node.children);
  }
  if (authoredClaim !== null) return authoredClaim;

  const runtime = doc.byId[ref.uuid];
  if (!runtime || runtime.page !== ref.page) return null;
  return opts.navigation || existingBlockId(runtime.raw, formatForBlock(ref.uuid)) === null ? ref.uuid : null;
}

/** `raw` with a durable `id` property added in the page's on-disk format.
 *  Markdown appends an `id:: <uuid>` trailer. ORG inserts/extends a
 *  `:PROPERTIES:`/`:id:`/`:END:` drawer at OG's canonical position — right after
 *  the title line and any SCHEDULED/DEADLINE planning lines (mirroring OG's
 *  `insert-property`, util/property.cljs). Writing markdown `id::` into an org
 *  file would BOTH render as visible body text and not be read back as the
 *  block's id (GH #25) — org MUST use the drawer. The caller guarantees the
 *  block has no id yet (see {@link existingBlockId}). */
export function rawWithBlockId(raw: string, uuid: string, format: Format): string {
  return editBlock(raw, format, { kind: "property", key: "id", value: uuid });
}

/** `raw` with an org drawer property set/updated/removed. Operates ONLY on the
 *  first `:PROPERTIES:` drawer in the canonical head region (title, planning,
 *  drawer, body — the same placement rawWithBlockId uses); body text and code
 *  blocks are never scanned. Removing the last property removes the drawer. */
export function orgRawWithProperty(raw: string, key: string, value: string | null): string {
  return editBlock(raw, "org", { kind: "property", key, value });
}

/** Ensure a block has a persistent id (assigned lazily, like OG) AND that it's
 *  durably on disk, returning the uuid — or null if it couldn't be saved
 *  (conflict/error). Used to make `((uuid))` references: the caller must not put
 *  a ref on the clipboard until the id is actually written, or quitting /
 *  resolving a conflict with "use disk version" would leave the ref dangling. */
export async function ensureBlockId(id: string): Promise<string | null> {
  const binding = captureBinding();
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return null;
  const fmt = formatForBlock(id);
  // Any existing id is the block's durable id — match its value (not just a UUID
  // shape), case-INSENSITIVELY (Rust's property("id") is case-insensitive, so an
  // `ID::` / `:ID:` from another editor counts), so we never write a SECOND id
  // that Rust then ignores → dangling copied ref.
  const existing = existingBlockId(node.raw, fmt);
  const uuid = existing ?? crypto.randomUUID();
  if (!existing) {
    setDoc("byId", id, "raw", rawWithBlockId(node.raw, uuid, fmt));
    markDirty(node.page, "save-block");
  }
  // Even a pre-existing id may not be on disk yet (added in-memory, not flushed);
  // flush and only hand back the uuid if the write actually landed.
  const ok = await flushPage(node.page);
  return ok && !isConflicted(node.page) && bindingCurrent(binding) ? uuid : null;
}

/** A live reference to a loaded block: its durable external UUID plus its exact
 * owner. The UUID can differ from the live store key until the page is reloaded. */
export function blockRef(id: string): LoadedBlockRef {
  const n = doc.byId[id];
  const owner = pageByName(n.page);
  return {
    uuid: blockExternalId(id) ?? n.id,
    page: n.page,
    pageKind: owner?.kind ?? "page",
    ...(owner?.id ? { path: owner.id } : {}),
  };
}

export const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Whether a block reference's id can name a block at all. OG resolves `((id))`
 *  only when `parse-uuid` accepts the id; `(((uuid)))` parses as the id `(uuid`,
 *  which it shows as an invalid reference (GH #589). */
export function isBlockRefUuid(id: string): boolean {
  return UUID_RE.test(id);
}

/** Stamp exactly `committed`, flush, and return the stamped id once it is on
 * disk. A block that already carries a different id resolves null. */
async function stampBlockId(id: string, committed: string): Promise<string | null> {
  const binding = captureBinding();
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return null;
  const fmt = formatForBlock(id);
  const existing = existingBlockId(node.raw, fmt);
  if (existing && existing !== committed) return null;
  const uuid = existing ?? committed;
  if (!existing) {
    setDoc("byId", id, "raw", rawWithBlockId(node.raw, uuid, fmt));
    markDirty(node.page, "save-block");
  }
  const ok = await flushPage(node.page);
  return ok && !isConflicted(node.page) && bindingCurrent(binding) && doc.byId[id]?.page === node.page
    && existingBlockId(doc.byId[id].raw, fmt) === uuid ? uuid : null;
}

/** The ref a saved session should carry for `ref`: an ID-less block gains its
 * position path, because its runtime key is only a locator that a restart may
 * hand to a different block; an identified block is named by its `id::`, and an
 * unloaded or unresolvable target keeps its ref unchanged. Navigation never
 * writes an `id::` (OG writes one only when a reference is created); this is how
 * a zoomed route survives a restart without it. Cost: O(page blocks). */
export function blockPositionRef<T extends LoadedBlockRef>(ref: T): T {
  if (ref.blockPos) return ref;
  const id = resolveBlockRef(ref, { navigation: true });
  const node = id ? doc.byId[id] : undefined;
  if (!id || !node) return ref;
  const authored = existingBlockId(node.raw, formatForBlock(id));
  if (authored !== null) return authored === ref.uuid ? ref : { ...ref, uuid: authored };
  const owner = pageByName(ref.page);
  const pos = owner ? positionOf(owner.roots, id) : null;
  return pos ? { ...ref, blockPos: pos } : ref;
}

/** A restored position ref once its block is found: the same ref carrying the
 * block's live identity instead of the position, so later edits that shift
 * siblings cannot move it. Null while the target is not (yet) resolvable. */
export function settleBlockRef(ref: LoadedBlockRef): LoadedBlockRef | null {
  if (!ref.blockPos) return ref;
  const id = resolveBlockRef(ref);
  if (!id) return null;
  const { blockPos: _pos, ...rest } = ref;
  return { ...rest, uuid: blockExternalId(id) ?? id };
}

/** Find the block that `externalId` names on `page` (exact `path` if given),
 * loading its page if needed; `uuid` is the caller's runtime locator and the
 * default `externalId`. Resolution goes through {@link resolveBlockRef}, so an
 * authored id wins and a runtime key is only a fallback for an ID-less block
 * (GH #373). Backend read errors reject; a missing, stale or unwritable
 * target resolves false. `externalId` is the ID to persist, stamped exactly. Without a callback,
 * flush the target page and resolve true only when its ID reaches disk.
 * With `insertReference`, validate the ID first, then call it synchronously to
 * edit the source and return its page name (null aborts without a write). The
 * grouped save writes the target ID before the source reference, so a crash
 * between files leaves at most an unreferenced ID. A failed save resolves false
 * and leaves both edits visible for conflict resolution. Cost: one page lookup
 * and one page or grouped save, possibly with other pending edits on the pages. */
export async function persistBlockRefTarget(
  uuid: string,
  page: string,
  kind: PageKind,
  path?: string,
  externalId: string = uuid,
  insertReference?: () => string | null,
): Promise<boolean> {
  const owner = bindingOwner();
  const ref: LoadedBlockRef = { uuid: externalId, page, pageKind: kind, ...(path ? { path } : {}) };
  if (!resolveBlockRef(ref)) {
    const result = await readOwned(owner, path
      ? backend().getPageByPath(path)
      : backend().getPage(page, kind));
    if (result.kind === "stale") return false;
    // A refused load leaves another file in the slot: nothing safe to stamp.
    if (result.value && ensurePageLoaded(result.value)) return false;
  }
  // Re-check: a concurrent navigation may have loaded the page meanwhile, or the
  // cache may have been rebuilt (external change) and reassigned the block a new
  // uuid — in which case there's nothing safe to stamp.
  const id = resolveBlockRef(ref);
  if (!id || !owner() || !blockWritable(id)) return false;
  // `externalId` is the value the caller has committed (or will commit) as the
  // reference, so it is stamped exactly, even when it equals the runtime key.
  if (!insertReference) return (await stampBlockId(id, externalId)) === externalId;
  const target = doc.byId[id];
  const fmt = formatForBlock(id);
  const existing = existingBlockId(target.raw, fmt);
  const stableId = existing ?? externalId;
  if (stableId !== externalId) return false;
  const sourcePage = insertReference();
  if (!sourcePage || !owner()) return false;
  if (!existing) setDoc("byId", id, "raw", rawWithBlockId(target.raw, stableId, fmt));
  // A reference may reach disk only after its target ID does. orderedMembers
  // writes the dependency destination first, independent of page name order.
  const saved = await persistTogether([sourcePage, target.page], "save-block", [[sourcePage, target.page]]);
  return saved && !isConflicted(sourcePage) && !isConflicted(target.page)
    && owner() && doc.byId[id]?.page === target.page
    && existingBlockId(doc.byId[id].raw, fmt) === stableId;
}
