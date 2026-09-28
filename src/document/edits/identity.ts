import { type Format, type PageKind } from "../../types";
import { doc, formatForBlock, pageByName, setDoc } from "../model";
import { captureBinding, stillBound } from "../../binding";
import { blockWritable } from "./properties";
import { markDirty, flushPage } from "../save/engine";
import { backend } from "../../backend";
import { ensurePageLoaded } from "../workingSet";
import { orgBlockDrawerRange } from "../../editor/properties";

/** The block's existing durable `id` — a markdown `id:: <uuid>` trailer or an
 *  org `:PROPERTIES:` drawer `:id: <uuid>` line — case-insensitively, or null.
 *  Format-aware because in ORG `id:: x` is plain body text, NOT a property (lsdoc
 *  reads the drawer, not a `key::` line); so an org block's real id lives in its
 *  `:PROPERTIES:` drawer and must be matched there (GH #25). */
export function existingBlockId(raw: string, format: Format): string | null {
  if (format !== "org") return /(?:^|\n)id:: *(\S+)/i.exec(raw)?.[1] ?? null;
  const lines = raw.split("\n");
  const drawer = orgBlockDrawerRange(lines);
  if (!drawer) return null;
  for (const line of lines.slice(drawer[0] + 1, drawer[1])) {
    const id = /^\s*:id:\s*(\S+)/i.exec(line)?.[1];
    if (id) return id;
  }
  return null;
}

/** The identity other blocks and persisted UI state must use for a loaded node.
 * A freshly-created node keeps its transient `b…` store key for the whole live
 * session even after Copy block ref writes a UUID property into `raw`; external
 * references must follow that property while render/edit paths keep the key. */
export function blockExternalId(id: string): string | null {
  const node = doc.byId[id];
  if (!node) return null;
  return existingBlockId(node.raw, formatForBlock(id)) ?? node.id;
}

export interface LoadedBlockRef {
  uuid: string;
  page: string;
  pageKind: PageKind;
  path?: string;
}

/** Resolve a durable external UUID back to the current live store key. The page
 * descriptor is part of the identity: even a direct `byId[uuid]` hit is rejected
 * when it belongs to another page kind or physical path. */
export function resolveBlockRef(ref: LoadedBlockRef): string | null {
  const owner = pageByName(ref.page);
  if (
    !owner
    || owner.kind !== ref.pageKind
    || (ref.path !== undefined && owner.id !== ref.path)
  ) return null;

  const matches = (id: string): boolean => {
    const node = doc.byId[id];
    return !!node && node.page === ref.page && blockExternalId(id) === ref.uuid;
  };
  if (matches(ref.uuid)) return ref.uuid;

  const stack = [...owner.roots];
  const seen = new Set<string>();
  while (stack.length) {
    const id = stack.pop()!;
    if (seen.has(id)) continue;
    seen.add(id);
    const node = doc.byId[id];
    if (!node || node.page !== ref.page) continue;
    if (matches(id)) return id;
    stack.push(...node.children);
  }
  return null;
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
  if (format !== "org") return `${raw}\nid:: ${uuid}`;
  const lines = raw.split("\n");
  const drawer = orgBlockDrawerRange(lines);
  if (drawer) {
    const [, end] = drawer;
    // Extend the existing drawer: insert the id line just before :END:.
    lines.splice(end, 0, `:id: ${uuid}`);
    return lines.join("\n");
  }
  // No drawer: title, SCHEDULED*, DEADLINE*, :PROPERTIES: drawer, rest-of-body —
  // OG groups planning lines above the drawer (util/property.cljs insert-property).
  const [title, ...rest] = lines;
  let planEnd = 0;
  while (planEnd < rest.length && /^\s*(?:SCHEDULED|DEADLINE):\s*</i.test(rest[planEnd])) planEnd++;
  return [title, ...rest.slice(0, planEnd), ":PROPERTIES:", `:id: ${uuid}`, ":END:", ...rest.slice(planEnd)].join("\n");
}

/** `raw` with an org drawer property set/updated/removed. Operates ONLY on the
 *  first `:PROPERTIES:` drawer in the canonical head region (title, planning,
 *  drawer, body — the same placement rawWithBlockId uses); body text and code
 *  blocks are never scanned. Removing the last property removes the drawer. */
export function orgRawWithProperty(raw: string, key: string, value: string | null): string {
  const lines = raw.split("\n");
  const drawer = orgBlockDrawerRange(lines);
  const keyRe = new RegExp(`^:${key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:\\s*`, "i");
  if (drawer) {
    const [start, end] = drawer;
    // Update in place so an existing drawer key keeps its position (GH #216);
    // only a new key appends.
    const inner = lines.slice(start + 1, end);
    const at = inner.findIndex((l) => keyRe.test(l.trim()));
    if (value !== null) {
      const line = `:${key}: ${value}`;
      if (at >= 0) inner[at] = line;
      else inner.push(line);
    } else if (at >= 0) {
      inner.splice(at, 1);
    }
    if (inner.length === 0) {
      // Drawer emptied: drop it entirely.
      return [...lines.slice(0, start), ...lines.slice(end + 1)].join("\n");
    }
    return [...lines.slice(0, start + 1), ...inner, ...lines.slice(end)].join("\n");
  }
  if (value === null) return raw; // nothing to remove
  // No drawer yet: title, SCHEDULED*, DEADLINE*, drawer, rest (rawWithBlockId's rule).
  const [title, ...rest] = lines;
  let planEnd = 0;
  while (planEnd < rest.length && /^\s*(?:SCHEDULED|DEADLINE):\s*</i.test(rest[planEnd])) planEnd++;
  return [
    title,
    ...rest.slice(0, planEnd),
    ":PROPERTIES:",
    `:${key}: ${value}`,
    ":END:",
    ...rest.slice(planEnd),
  ].join("\n");
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
  return ok && stillBound(binding) ? uuid : null;
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

/** Ensure a block has a durable external UUID synchronously, while deliberately
 * leaving its live store key unchanged. Existing ids win; otherwise a fresh
 * transient key receives a UUID in the page's Markdown/Org property syntax. */
export function ensureStableBlockId(id: string): string | null {
  const node = doc.byId[id];
  if (!node || !blockWritable(id)) return null;
  const fmt = formatForBlock(id);
  const existing = existingBlockId(node.raw, fmt);
  if (existing) return existing;
  const uuid = UUID_RE.test(id) ? id : crypto.randomUUID();
  setDoc("byId", id, "raw", rawWithBlockId(node.raw, uuid, fmt));
  markDirty(node.page, "save-block");
  // Persist now, not on the 400ms debounce: the user may quit right after
  // parking the block, and a pending timer is lost when the webview closes.
  void flushPage(node.page);
  return uuid;
}

/** Like `blockRef`, but first persists the block's `id::` so the reference
 *  resolves after a restart. Used for parking a block durably: the right sidebar,
 *  a new tab, and zoom all stamp `id::` so the spot survives a relaunch (Martin's
 *  call — he wants these to persist; the `id::` is harmless in the file and is
 *  stripped from clipboard copies anyway, see `blockSubtreeMarkdown`). */
export function persistentBlockRef(id: string): LoadedBlockRef {
  ensureStableBlockId(id);
  return blockRef(id);
}

/** Make a freshly-inserted `((uuid))` reference durable: ensure the TARGET block
 *  (which may live on a page that isn't loaded — block search spans the whole
 *  graph) carries `id:: uuid` on disk, so the ref still resolves after a restart.
 *  The owning page is loaded only if absent (`ensurePageLoaded` never clobbers
 *  unsaved edits). A no-op if the block already has an `id::`. Fire-and-forget:
 *  the ref resolves in-session via the in-memory uuid even before this lands. */
export async function persistBlockRefTarget(
  uuid: string,
  page: string,
  kind: PageKind,
  path?: string,
): Promise<void> {
  const binding = captureBinding();
  const ref: LoadedBlockRef = { uuid, page, pageKind: kind, ...(path ? { path } : {}) };
  if (!resolveBlockRef(ref)) {
    const dto = path
      ? await backend().getPageByPath(path)
      : await backend().getPage(page, kind);
    if (!stillBound(binding)) return;
    if (dto) ensurePageLoaded(dto);
  }
  // Re-check: a concurrent navigation may have loaded the page meanwhile, or the
  // cache may have been rebuilt (external change) and reassigned the block a new
  // uuid — in which case there's nothing safe to stamp.
  const id = resolveBlockRef(ref);
  if (id) ensureStableBlockId(id);
}
