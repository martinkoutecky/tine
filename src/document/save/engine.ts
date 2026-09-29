import { pageByName, setPageId, doc } from "../model";
import { createSignal } from "solid-js";
import { bumpDataRev, bumpPageInventoryRev } from "../../graphSession";
import { type ClipboardSourcePage } from "../../clipboard";
import { captureBinding, type Binding, stillBound } from "../../binding";
import { pageToDto, appendAliasDraft, aliasDraftBlocks, replaceLandedAliasDraft } from "../convert";
import type { BlockDto, PageDto, PageKind } from "../../types";
import { backend, saveOnePage, type SavePageEntry } from "../../backend";
import { forgetPage, reloadPage, loadSingle, rekeyPageIdentityByPath } from "../workingSet";
import { editingId } from "../../editorController";
import { pagePropertyEntries } from "../../editor/properties";
import { graphOwner, readOwned } from "../../owned";
import { dismissToast, pushToast } from "../../toasts";
import { openUnsavedRecovery } from "../../unsavedRecovery";
import { errorFamily } from "../../errorFamily";
import { describeSavePlatformStep, readSavePlatformStep, type SavePlatformStep } from "../../savePlatformStep";
import { graphRewriteFrozen } from "../graphRewriteState";
import type { EditKind, EditKinds } from "../../editKind";
import { adoptFoldedPageHeader } from "../edits/properties";

type IntentKinds = EditKind | EditKinds;
const kindLedger = new Map<string, EditKind[]>();
const titleIdentityIntents = new Set<string>();
/** Only explicit edits to the page's own title need an extra exact-path read
 * when the title is removed; ordinary saves stay page-bounded. */
export function noteTitleIdentityIntent(name: string): void { titleIdentityIntents.add(name); }
function noteKinds(name: string, kinds: IntentKinds): void {
  const pending = kindLedger.get(name) ?? [];
  for (const kind of typeof kinds === "string" ? [kinds] : kinds)
    if (!pending.includes(kind)) pending.push(kind);
  kindLedger.set(name, pending);
}
function pendingKinds(name: string, creating: boolean): EditKinds {
  const kinds = [...(kindLedger.get(name) ?? [])];
  if (creating && !kinds.includes("create-page")) kinds.unshift("create-page");
  // Every caller-facing dirty marker requires a kind, so an empty ledger means the
  // engine itself re-dirtied the page to retry it (a released group member). That
  // is a whole-page write with no finer intent: replace-page. Never throw here:
  // a throw in the save path would strand the user's edit unsaved.
  if (!kinds.length) kinds.push("replace-page");
  return [kinds[0], ...kinds.slice(1)];
}
function restoreKinds(name: string, previous: EditKinds): void {
  const newer = kindLedger.get(name) ?? [];
  kindLedger.set(name, [...previous, ...newer.filter((kind) => !previous.includes(kind))]);
}

let aliasDraftRouteHandler: ((name: string, kind: PageDto["kind"]) => void) | null = null;
export function installAliasDraftRouteHandler(handler: (name: string, kind: PageDto["kind"]) => void): void {
  aliasDraftRouteHandler = handler;
}

export async function deletePageOnDisk(name: string, kind: PageKind, expectedPath?: string): Promise<void> {
  if (expectedPath) await backend().deletePage(name, kind, expectedPath);
  else await backend().deletePage(name, kind);
}

export type CreatePageRefusalReason =
  | "name-mismatch" | "page-conflicted" | "page-dirty" | "page-saving"
  | "stale-binding" | "alias" | "page-rebound" | "graph-changed" | "graph-rewrite";

/** Local precondition refusal. The backend's fixed `conflict` token is reserved
 * for a file that changed on disk. */
export class CreatePageRefusal extends Error {
  constructor(readonly reason: CreatePageRefusalReason) {
    super(`create-page:${reason}`);
    this.name = "CreatePageRefusal";
  }
}

/** Save a newly authored DTO through the same binding and save-state owner as edits. */
export async function createPage(
  name: string,
  dto: PageDto,
  options: { id?: string; baseRev?: string | null; bindingGeneration?: number } = {},
): Promise<string> {
  const binding = captureBinding();
  const token = graphToken;
  const generation = pageInstanceGeneration(name);
  if (graphRewriteFrozen()) throw new CreatePageRefusal("graph-rewrite");
  if (dto.name !== name) throw new CreatePageRefusal("name-mismatch");
  if (isConflicted(name)) throw new CreatePageRefusal("page-conflicted");
  if (isDirty(name)) throw new CreatePageRefusal("page-dirty");
  if (isSaving(name)) throw new CreatePageRefusal("page-saving");
  if (options.bindingGeneration !== undefined && options.bindingGeneration !== binding.backendGeneration)
    throw new CreatePageRefusal("stale-binding");
  const resolved = options.id ? null : await backend().resolvePage(name, dto.kind);
  if (!stillBound(binding) || token !== graphToken) throw new CreatePageRefusal("graph-changed");
  if (graphRewriteFrozen()) throw new CreatePageRefusal("graph-rewrite");
  if (resolved?.kind === "alias") throw new CreatePageRefusal("alias");
  const id = options.id ?? resolved!.id;
  if (!stillBound(binding) || token !== graphToken) throw new CreatePageRefusal("graph-changed");
  if (pageInstanceGeneration(name) !== generation) throw new CreatePageRefusal("page-rebound");
  const wasTombstoned = deletedPages.delete(name); // an explicit create supersedes a completed delete
  try {
    const rev = await saveOnePage(backend(), { id, page: dto, baseRev: options.baseRev ?? null, force: false,
      kinds: [options.baseRev == null ? "create-page" : "replace-page"] }, binding.backendGeneration);
    if (!stillBound(binding) || token !== graphToken) throw new CreatePageRefusal("graph-changed");
    if (graphRewriteFrozen()) throw new CreatePageRefusal("graph-rewrite");
    if (pageInstanceGeneration(name) === generation) {
      setPageId(name, id);
      setBaseRev(name, rev);
      clearConflict(name);
      forgetSaveFailure(name);
    }
    if (options.baseRev == null) bumpPageInventoryRev();
    bumpDataRev();
    return rev;
  } catch (error) {
    if (wasTombstoned && stillBound(binding) && token === graphToken) deletedPages.add(name);
    if (stillBound(binding) && token === graphToken && pageInstanceGeneration(name) === generation
        && errorFamily(error) === "conflict")
      markConflict(name, { kind: "disk-changed" }, (error as { diskRev?: string | null }).diskRev);
    throw error;
  }
}

// Pages that failed to save because the file changed on disk (external edit /
// Syncthing). Surfaced as a banner; the user resolves with reload or overwrite.
export type ConflictReason =
  | { kind: "disk-changed" | "repeated" | "alias-owner-busy"; observedRev?: string | null }
  | { kind: "released"; partner: string; observedRev?: string | null };
export const [conflictReasons, setConflictReasons] = createSignal<Record<string, ConflictReason>>({});
export const conflicts = () => Object.keys(conflictReasons());
export function conflictReason(name: string): ConflictReason | undefined {
  return conflictReasons()[name];
}
export function markConflict(name: string, reason: ConflictReason = { kind: "disk-changed" }, observedRev?: string | null) {
  setConflictReasons({ ...conflictReasons(), [name]: observedRev === undefined ? reason : { ...reason, observedRev } });
  noteRisk(name);
}
export function clearConflict(name: string) {
  const next = { ...conflictReasons() };
  delete next[name];
  setConflictReasons(next);
  noteRisk(name);
}
/** Told when a page starts or stops holding edits that cannot currently be saved
 *  (a conflict or a failed save), so it can keep a crash-surviving copy (og ADR
 *  0061). One keeper; installing replaces it. */
type DraftKeeper = (name: string, atRisk: boolean) => void;
let draftKeeper: DraftKeeper | null = null;
export function installDraftKeeper(keeper: DraftKeeper | null) { draftKeeper = keeper; }
function noteRisk(name: string) {
  draftKeeper?.(name, !!conflictReasons()[name] || lastSaveFailure.has(name));
}
export function isConflicted(name: string): boolean {
  return !!conflictReasons()[name];
}

// A generation identifies one exact loaded page instance. It is deliberately
// frontend-only and monotonic across resets: a later page with the same name and
// path must never satisfy a cut payload captured from an evicted/deleted/rebound
// instance. Stage B uses this at its durable-retirement boundary.
let pageInstanceClock = 0;
export const pageInstanceGenerations = new Map<string, number>();

export function activatePageInstance(name: string): number {
  const generation = ++pageInstanceClock;
  pageInstanceGenerations.set(name, generation);
  return generation;
}

export function retirePageInstance(name: string): void {
  ++pageInstanceClock;
  pageInstanceGenerations.delete(name);
}

/** Current exact loaded-page generation, or null when that page is absent. */
export function pageInstanceGeneration(name: string): number | null {
  if (!pageByName(name)) return null;
  // Direct setDoc page seeding is supported by model tests and small embedded
  // surfaces; lazily bind it to the same invariant as loader-created pages.
  return pageInstanceGenerations.get(name) ?? activatePageInstance(name);
}
// ---------------------------------------------------------------------------
// Guard state (owned here; mutated only through the accessors below)
// ---------------------------------------------------------------------------

const dirty = new Set<string>();
// Per-page save baseline: the on-disk file rev the editor last loaded or saved.
// Sent on save so the backend conflicts against the version the editor actually
// has, not its own mutable cache (which the watcher can advance under us).
const baseRev = new Map<string, string | null>();
// Pages the user just deleted. A never-saved page can have a queued save with
// baseRev=null; without this, that save fires after the delete and the backend
// (missing file + null baseline = "new page") happily recreates it. While a name
// is tombstoned, saves for it are skipped; re-loading/creating the page clears it.
const deletedPages = new Set<string>();
// Alias drafts whose snapshot already landed at the end of their owner file
// while a later edit kept the draft open and conflicted (L13): the owner file
// and the copy it holds. A retry replaces that copy; it never appends again.
const landedAliasDrafts = new Map<string, { owner: string; blocks: BlockDto[]; generation: number | null }>();
/** What alias draft `name` (instance `generation`) writes to `owner`: its
 *  snapshot appended, or the copy that already landed replaced in place. Null
 *  refuses: the owner no longer ends with that copy (an external editor or a
 *  sync client changed it since), and appending again would duplicate it. */
function aliasOwnerPage(name: string, generation: number | null, owner: PageDto & { id?: string }, dto: PageDto): PageDto | null {
  const landed = landedAliasDrafts.get(name);
  if (!landed || landed.generation !== generation) return appendAliasDraft(owner, dto);
  return landed.owner === owner.id ? replaceLandedAliasDraft(owner, landed.blocks, dto) : null;
}
// Bumped whenever the working set is reset (graph switch). A save abandons its
// baseline update if the graph changed under it; resetSaveState also clears
// `dirty` so a stray queued save becomes a no-op.
let graphToken = 0;
// Per-page save queue: writes for one page run strictly one-after-another (never
// concurrently) and each runs against the LATEST store state.
type SaveResult = boolean | "deferred";
const saveChain = new Map<string, Promise<SaveResult>>();
const lastSaveFailure = new Map<string, string>();
const saveFailureToasts = new Map<string, number>();
/** Say once per failure family that `name` did not save, and keep saying it (a
 * sticky toast with the way to the draft) until it saves or leaves (GH #540). */
function reportSaveFailure(name: string, family: string, message: string) {
  if (lastSaveFailure.get(name) === family) return;
  forgetSaveFailure(name);
  lastSaveFailure.set(name, family);
  noteRisk(name);
  saveFailureToasts.set(name, pushToast(message, "error", { sticky: true, action: { label: "Review unsaved", run: openUnsavedRecovery } }));
}
function forgetSaveFailure(name: string) {
  if (lastSaveFailure.delete(name)) noteRisk(name);
  const toast = saveFailureToasts.get(name);
  saveFailureToasts.delete(name);
  if (toast !== undefined) dismissToast(toast);
}
let saveTimer: ReturnType<typeof setTimeout> | null = null;
let saveBurstStart: number | null = null;
let dataRevTimer: ReturnType<typeof setTimeout> | null = null;
const assetWriteChain = new Set<Promise<boolean>>();
export type TransferEdge = readonly [source: string, destination: string];
export interface SaveGroup {
  members: Set<string>;
  edges: Set<string>;
  forced: Map<string, ConflictReason>;
  state: "open" | "sealed";
  pending?: Promise<boolean>;
  request?: Promise<boolean>;
  redirect?: SaveGroup;
  cancelled?: boolean;
  waiters: Array<(ok: boolean) => void>;
}
const groupOf = new Map<string, SaveGroup>();
const sealedGroups = new Set<SaveGroup>();
const saveAttempts = new Map<string, number>();
function noteSaveAttempt(name: string): void {
  saveAttempts.set(name, (saveAttempts.get(name) ?? 0) + 1);
}
function decidedConflict(g: SaveGroup, name: string): boolean {
  const reason = conflictReason(name);
  return !!reason && g.forced.get(name) === reason;
}
const deletingGroupMembers = new Set<string>();
const [groupRevision, setGroupRevision] = createSignal(0);
function changedGroups() { setGroupRevision((n) => n + 1); }
export function group(name: string): SaveGroup | undefined {
  groupRevision();
  const g = groupOf.get(name);
  return g?.redirect ?? g;
}
export function groupedPages(): Iterable<string> { groupRevision(); return groupOf.keys(); }
export function savingPages(): Iterable<string> { return saveChain.keys(); }
export async function reserveGroupMemberDeletion(name: string): Promise<() => void> {
  deletingGroupMembers.add(name);
  await Promise.all([...sealedGroups].filter((g) => g.members.has(name)).map((g) => g.request));
  return () => { deletingGroupMembers.delete(name); };
}
export function waitingOn(name: string): string[] {
  const g = group(name);
  if (!g) return [];
  return [...g.members].filter((member) => member !== name && !isConflicted(member));
}
export function waitingFor(name: string): string[] {
  const g = group(name);
  if (!g) return [];
  return [...g.members].filter((member) => member !== name && isConflicted(member) && !decidedConflict(g, member));
}
export function moveConflict(pages: Iterable<string>): string | undefined {
  for (const name of pages) {
    if (isConflicted(name)) return name;
    const blocked = [...(group(name)?.members ?? [])].find(isConflicted);
    if (blocked) return blocked;
  }
}
export function refuseConflictedMove(pages: Iterable<string>): boolean {
  const blocked = moveConflict(pages);
  if (!blocked) return false;
  pushToast(`Resolve the conflict on “${blocked}” first.`, "error");
  return true;
}

function mergeInto(target: SaveGroup, other: SaveGroup): void {
  if (target === other || other.redirect) return;
  for (const member of other.members) {
    target.members.add(member);
    groupOf.set(member, target);
  }
  for (const edge of other.edges) target.edges.add(edge);
  for (const [member, reason] of other.forced) {
    if (!decidedConflict(target, member)) target.forced.set(member, reason);
  }
  target.waiters.push(...other.waiters);
  other.waiters.length = 0;
  other.redirect = target;
  changedGroups();
}

function registerGroup(pages: Iterable<string>, edges: Iterable<TransferEdge>): SaveGroup {
  const names = new Set(pages);
  const existing = [...names].map((name) => group(name)).filter((g): g is SaveGroup => !!g && g.state === "open");
  const target = existing[0] ?? { members: new Set<string>(), edges: new Set<string>(), forced: new Map<string, ConflictReason>(), state: "open" as const, waiters: [] };
  for (const other of existing) mergeInto(target, other);
  for (const name of names) {
    target.members.add(name);
    groupOf.set(name, target);
  }
  for (const edge of edges) target.edges.add(JSON.stringify(edge));
  changedGroups();
  return target;
}

/** Register one user's edit as one durable request. The returned promise settles
 * when the debounced group flush completes. */
export function persistTogether(pages: Iterable<string>, kinds: IntentKinds, edges: Iterable<TransferEdge> = []): Promise<boolean> {
  const names = [...new Set(pages)].filter((name) => {
    const page = pageByName(name);
    return !!page && !page.guide && !page.readOnly;
  });
  if (!names.length) return Promise.resolve(true);
  const g = registerGroup(names, edges);
  for (const name of names) { dirty.add(name); noteKinds(name, kinds); }
  scheduleSave();
  return new Promise<boolean>((resolve) => g.waiters.push(resolve));
}

function dissolveGroup(g: SaveGroup): void {
  g.cancelled = true;
  for (const name of g.members) if (groupOf.get(name) === g) groupOf.delete(name);
  sealedGroups.delete(g);
  changedGroups();
}

/** Choosing disk for one member releases every other member for an explicit
 * decision. This runs only after the disk read is still bound to this instance. */
export function releaseGroup(name: string): void {
  const g = group(name);
  if (!g) return;
  dissolveGroup(g);
  for (const member of g.members) {
    if (member === name) continue;
    dirty.add(member);
    markConflict(member, { kind: "released", partner: name });
  }
  for (const resolve of g.waiters) resolve(false);
  g.waiters.length = 0;
}

function orderedMembers(g: SaveGroup): string[] {
  const names = [...g.members];
  const adjacency = new Map(names.map((name) => [name, new Set<string>()]));
  const linked = new Set<string>();
  for (const raw of g.edges) {
    const [source, destination] = JSON.parse(raw) as [string, string];
    if (adjacency.has(source) && adjacency.has(destination) && source !== destination) {
      adjacency.get(source)!.add(destination);
      linked.add(source); linked.add(destination);
    }
  }
  let clock = 0;
  const number = new Map<string, number>(), low = new Map<string, number>();
  const stack: string[] = [], onStack = new Set<string>(), components: string[][] = [];
  const visit = (name: string) => {
    number.set(name, clock); low.set(name, clock++);
    stack.push(name); onStack.add(name);
    for (const next of adjacency.get(name)!) {
      if (!number.has(next)) { visit(next); low.set(name, Math.min(low.get(name)!, low.get(next)!)); }
      else if (onStack.has(next)) low.set(name, Math.min(low.get(name)!, number.get(next)!));
    }
    if (low.get(name) === number.get(name)) {
      const component: string[] = [];
      let next: string;
      do { next = stack.pop()!; onStack.delete(next); component.push(next); } while (next !== name);
      components.push(component);
    }
  };
  for (const name of names) if (!number.has(name)) visit(name);
  const componentOf = new Map(components.flatMap((members, i) => members.map((name) => [name, i] as const)));
  const successors = components.map(() => new Set<number>());
  for (const [source, destinations] of adjacency) for (const destination of destinations) {
    const from = componentOf.get(source)!, to = componentOf.get(destination)!;
    if (from !== to) successors[from].add(to);
  }
  const pending = new Set(components.map((_, i) => i).filter((i) => components[i].some((name) => linked.has(name))));
  const result: string[] = [];
  while (pending.size) {
    const ready = [...pending].filter((i) => [...successors[i]].every((to) => !pending.has(to)));
    for (const i of ready) { result.push(...components[i]); pending.delete(i); }
  }
  return result.concat(components.filter((members) => members.every((name) => !linked.has(name))).flat());
}

function validGroupMembers(g: SaveGroup, binding: Binding, token: number, generations: Map<string, number | null>): boolean {
  return !g.cancelled && stillBound(binding) && token === graphToken
    && [...g.members].every((name) => pageInstanceGeneration(name) === generations.get(name) && !deletedPages.has(name));
}

function abortGroup(g: SaveGroup): false {
  if (!g.cancelled) {
    g.state = "open";
    sealedGroups.delete(g);
    for (const name of g.members) dirty.add(name);
    changedGroups();
  }
  return false;
}

function resolveSaveMember(name: string, kind: PageKind) {
  return backend().resolvePage(name, kind);
}

function failGroup(g: SaveGroup, failure: { index: number; family: string; diskRev?: string | null; undoFailed: string[]; publicationErrors?: string[]; operation?: string; osError?: number }, order: string[], entryPaths: string[] = []): boolean {
  g.state = "open";
  sealedGroups.delete(g);
  for (const name of g.members) dirty.add(name);
  const successor = [...g.members].map((name) => groupOf.get(name)).find((other) => other && other !== g && other.state === "open");
  if (successor) mergeInto(successor, g);
  const family = failure.family;
  if (family === "repeated" || (family === "twin" && !pageByName(order[failure.index])?.id)) {
    for (const name of g.members) markConflict(name, { kind: "repeated" });
    pushToast("Couldn't save these pages together: two entries target the same file.", "error");
    return false;
  }
  const culprit = order[failure.index];
  if (culprit) {
    if (family === "alias-owner-busy") markConflict(culprit, { kind: "alias-owner-busy" });
    else if (["conflict", "deleted", "twin", "read-only", "invalid-target"].includes(family))
      markConflict(culprit, { kind: "disk-changed" }, family === "deleted" ? null : failure.diskRev);
    else reportSaveFailure(culprit, family, `Couldn't save “${culprit}” — ${family}${describeSavePlatformStep(readSavePlatformStep(failure))}.`);
  }
  for (const path of failure.undoFailed) {
    const index = entryPaths.indexOf(path);
    if (index >= 0) markConflict(order[index]);
    else pushToast(`Rollback incomplete for ${path}; inspect the file on disk before retrying.`, "error");
  }
  for (const path of failure.publicationErrors ?? []) {
    const index = entryPaths.indexOf(path);
    if (index >= 0) markConflict(order[index]);
    pushToast(`Publication incomplete for ${path}; inspect the file on disk before retrying.`, "error");
  }
  changedGroups();
  return false;
}

async function runGroup(g: SaveGroup, request: Promise<boolean>): Promise<boolean> {
  if (g.cancelled) return false;
  if (g.redirect) return enqueueGroup(g.redirect);
  const binding = captureBinding(), token = graphToken;
  // Stay open while a sealed predecessor is in flight; a failed predecessor
  // merges into this group before this group's seal point.
  while (true) {
    const predecessors = [...sealedGroups].filter((older) => older !== g && [...g.members].some((name) => older.members.has(name)));
    if (!predecessors.length) break;
    await Promise.all(predecessors.map((older) => older.request));
    if (g.redirect) return enqueueGroup(g.redirect);
    if (!stillBound(binding) || token !== graphToken) return false;
  }
  if (g.cancelled || [...g.members].some((name) => deletedPages.has(name) || deletingGroupMembers.has(name))) return false;
  if ([...g.members].some((name) => isConflicted(name) && !decidedConflict(g, name))) return false;
  // Seal atomically: no await between capturing the remaining single-page tails
  // and registering this request as every member's current chain entry.
  g.state = "sealed";
  g.pending = undefined;
  sealedGroups.add(g);
  const tails = [...g.members].map((name) => saveChain.get(name)).filter((tail): tail is Promise<SaveResult> => !!tail);
  const generations = new Map([...g.members].map((name) => [name, pageInstanceGeneration(name)]));
  for (const name of g.members) {
    noteSaveAttempt(name);
    saveChain.set(name, request);
  }
  changedGroups();
  await Promise.all(tails);
  if (!validGroupMembers(g, binding, token, generations)) return abortGroup(g);
  const order = orderedMembers(g);
  const ids = new Map<string, { id: string; owner?: PageDto & { id?: string; rev?: string | null }; ownerGeneration?: number | null }>();
  for (const name of order) {
    const page = pageByName(name);
    if (!page) return abortGroup(g);
    if (page.id) { ids.set(name, { id: page.id }); continue; }
    try {
      const resolved = await resolveSaveMember(name, page.kind);
      if (!validGroupMembers(g, binding, token, generations)) return abortGroup(g);
      if (resolved.kind === "alias") {
        const owner = resolved.owners[0] && await backend().getPageByPath(resolved.owners[0]);
        if (!stillBound(binding) || !validGroupMembers(g, binding, token, generations)) return abortGroup(g);
        if (!owner || owner.read_only || owner.guide) return failGroup(g, { index: order.indexOf(name), family: "alias-owner-busy", undoFailed: [] }, order);
        if (order.some((member) => member !== name && pageByName(member)?.id === owner.id))
          return failGroup(g, { index: order.indexOf(name), family: "repeated", undoFailed: [] }, order);
        if (isDirty(owner.name) || isSaving(owner.name) || isConflicted(owner.name)) {
          markConflict(name, { kind: "alias-owner-busy" });
          return abortGroup(g);
        }
        ids.set(name, { id: owner.id!, owner, ownerGeneration: pageInstanceGeneration(owner.name) });
      } else ids.set(name, { id: resolved.id });
    } catch (error) {
      if (!validGroupMembers(g, binding, token, generations)) return abortGroup(g);
      return failGroup(g, { index: order.indexOf(name), family: errorFamily(error), undoFailed: [] }, order);
    }
  }
  if (!validGroupMembers(g, binding, token, generations)) return abortGroup(g);
  const entries: SavePageEntry[] = [];
  const drafts: PageDto[] = [];
  for (const name of order) {
    const dto = pageToDto(name), target = ids.get(name)!;
    if (!dto) return abortGroup(g);
    const decision = decidedConflict(g, name) ? conflictReason(name) : undefined;
    const ownerPage = target.owner && aliasOwnerPage(name, generations.get(name) ?? null, target.owner, dto);
    if (target.owner && !ownerPage) return failGroup(g, { index: order.indexOf(name), family: "conflict", undoFailed: [] }, order);
    drafts.push(dto);
    entries.push({ id: target.id, page: ownerPage || dto,
      baseRev: decision?.observedRev !== undefined ? decision.observedRev : target.owner ? target.owner.rev ?? null : baseRev.get(name) ?? null,
      force: false,
      kinds: target.owner ? ["insert-blocks", "delete-page"] : pendingKinds(name, (baseRev.get(name) ?? null) === null) });
  }
  if (new Set(entries.map((entry) => entry.id)).size !== entries.length)
    return failGroup(g, { index: 0, family: "repeated", undoFailed: [] }, order);
  const forcedConflicts = new Map(g.forced);
  for (const name of order) { dirty.delete(name); kindLedger.delete(name); }
  try {
    const outcome = await backend().savePages(entries, binding.backendGeneration);
    if (!stillBound(binding) || !validGroupMembers(g, binding, token, generations)) {
      for (let i = 0; i < order.length; i++) restoreKinds(order[i], entries[i].kinds);
      return abortGroup(g);
    }
    if ("failed" in outcome) {
      for (let i = 0; i < order.length; i++) restoreKinds(order[i], entries[i].kinds);
      return failGroup(g, outcome.failed, order, entries.map((entry) => entry.id));
    }
    for (let i = 0; i < order.length; i++) {
      const name = order[i], target = ids.get(name)!;
      if (target.owner) {
        landedAliasDrafts.set(name, { owner: target.id, blocks: aliasDraftBlocks(drafts[i]), generation: generations.get(name) ?? null });
        continue;
      }
      setPageId(name, target.id);
      baseRev.set(name, outcome.ok[i]);
      const writtenHeader = entries[i].page.pre_block;
      if (writtenHeader) adoptFoldedPageHeader(name, writtenHeader);
      if (entries[i].baseRev === null) bumpPageInventoryRev();
      if (forcedConflicts.has(name) && conflictReason(name) === forcedConflicts.get(name)) clearConflict(name);
      forgetSaveFailure(name);
    }
    dissolveGroup(g);
    for (let i = 0; i < order.length; i++) {
      const name = order[i], target = ids.get(name)!;
      if (!target.owner) await settleSavedTitleIdentity(name, target.id, entries[i].page, outcome.ok[i]);
    }
    for (let i = 0; i < order.length; i++) {
      const name = order[i], target = ids.get(name)!;
      if (!target.owner) continue;
      if (dirty.has(name) || isDirty(target.owner.name) || isSaving(target.owner.name)
          || isConflicted(target.owner.name) || pageInstanceGeneration(target.owner.name) !== target.ownerGeneration) {
        markConflict(name, { kind: "alias-owner-busy" });
        continue;
      }
      const landed = { ...entries[i].page, rev: outcome.ok[i] };
      landedAliasDrafts.delete(name);
      forgetPage(name);
      reloadPage(landed);
      loadSingle(landed);
      aliasDraftRouteHandler?.(target.owner.name, target.owner.kind);
      pushToast(`Moved “${name}” into its alias owner “${target.owner.name}”.`, "info");
      bumpPageInventoryRev();
    }
    if ([...g.members].some((name) => dirty.has(name))) scheduleSave();
    return true;
  } catch (error) {
    if (!stillBound(binding) || token !== graphToken || g.cancelled) return false;
    for (let i = 0; i < order.length; i++) restoreKinds(order[i], entries[i].kinds);
    return failGroup(g, { index: 0, family: errorFamily(error), undoFailed: [] }, order);
  }
}

function enqueueGroup(input: SaveGroup): Promise<boolean> {
  const g = input.redirect ?? input;
  if (g.cancelled) return Promise.resolve(false);
  if (g.state === "sealed") return g.request ?? Promise.resolve(false);
  if (g.pending) return g.pending;
  let request!: Promise<boolean>;
  request = Promise.resolve().then(() => runGroup(g, request)).catch((error) => {
    if (g.cancelled || !sealedGroups.has(g) && ![...g.members].some((name) => groupOf.get(name) === g)) return false;
    return failGroup(g, { index: 0, family: errorFamily(error), undoFailed: [] }, orderedMembers(g));
  }).finally(() => {
    if (g.request === request) g.request = undefined;
    if (g.pending === request) g.pending = undefined;
    for (const name of g.members) if (saveChain.get(name) === request) saveChain.delete(name);
  });
  g.pending = request;
  g.request = request;
  void request.then((ok) => {
    for (const resolve of g.waiters) resolve(ok);
    g.waiters.length = 0;
  });
  return request;
}

// ---------------------------------------------------------------------------
// Accessors — store.ts mutations call these instead of touching the guards.
// ---------------------------------------------------------------------------

/** Does a page have unsaved (debounced) edits pending? */
export function isDirty(name: string): boolean {
  return dirty.has(name);
}
/** Mark a page dirty and schedule a debounced save. */
export function markDirty(name: string, kinds: IntentKinds) {
  const page = pageByName(name);
  if (page?.readOnly || page?.guide) return;
  dirty.add(name);
  noteKinds(name, kinds);
  scheduleSave();
}
/** Mark dirty WITHOUT scheduling — undo/redo restore batches several pages then
 *  schedules once. */
export function addDirty(name: string, kinds: IntentKinds) {
  const page = pageByName(name);
  if (page?.readOnly || page?.guide) return;
  dirty.add(name);
  noteKinds(name, kinds);
}
/** Pages with pending edits (so the working-set cap can pin them). */
export function dirtyPages(): Iterable<string> {
  return dirty;
}
/** How many pages hold edits that are pending or still being written — a
 *  count only, for the close-discard diagnostic (GH #540). O(dirty + saving). */
export function unsavedPageCount(): number {
  return new Set([...dirty, ...saveChain.keys()]).size;
}
export type UnsavedState = "Saving" | "Conflict" | "Not saved";
/** Every page whose edits are not yet on disk, with the loaded draft to recover
 *  (GH #540 recovery panel and close prompt). Read-only; O(dirty + saving +
 *  conflicts) plus one DTO conversion per page. */
export function unsavedDrafts(): { name: string; state: UnsavedState; path: string | null; page: PageDto | null;
  live: boolean; baseRev: string | null; observedRev: string | null }[] {
  const names = new Set([...dirty, ...saveChain.keys(), ...conflicts()]);
  return [...names].map((name) => {
    const reason = conflictReasons()[name];
    return {
      name,
      state: saveChain.has(name) ? "Saving" : reason ? "Conflict" : "Not saved",
      path: pageByName(name)?.id ?? null,
      page: pageToDto(name),
      // A live-draft conflict (og 8e) the resolver can take after a restart.
      live: reason?.kind === "disk-changed" && !group(name),
      baseRev: baseRev.get(name) ?? null,
      observedRev: reason?.observedRev ?? null,
    };
  });
}
/** Is a save currently queued/in flight for this page? (a cross-page move must
 *  flush the source first so it isn't written after being emptied). */
export function isSaving(name: string): boolean {
  return saveChain.has(name);
}

/** Track an optimistic asset write so flushAll/app-close waits for the bytes to
 *  land before letting the process exit. The caller still owns success/failure
 *  handling for any UI/store rollback. */
export function trackAssetWrite<T>(write: Promise<T>): Promise<T> {
  let tracked: Promise<boolean>;
  tracked = write.then(
    () => true,
    () => false
  ).finally(() => {
    assetWriteChain.delete(tracked);
  });
  assetWriteChain.add(tracked);
  return write;
}
/** Record a page's load/save baseline rev (set on load and after each save). */
export function setBaseRev(name: string, rev: string | null) {
  baseRev.set(name, rev);
}
/** The revision page `name` was last loaded or saved at; `undefined` when unknown. O(1). */
export function baseRevFor(name: string): string | null | undefined {
  return baseRev.get(name);
}
/** Transfer one exact loaded file's save ownership after its effective title
 * changes. A pending old-name save is allowed to retire; any newer dirty intent
 * is scheduled under the new name and remains revision guarded. */
export function rekeyPageSaveState(oldName: string, newName: string, rev: string | null): void {
  if (dirty.delete(oldName)) dirty.add(newName);
  const kinds = kindLedger.get(oldName);
  kindLedger.delete(oldName);
  if (kinds) kindLedger.set(newName, kinds);
  baseRev.delete(oldName);
  baseRev.set(newName, rev);
  forgetSaveFailure(oldName);
  if (titleIdentityIntents.delete(oldName) && dirty.has(newName)) titleIdentityIntents.add(newName);
  const generation = pageInstanceGenerations.get(oldName);
  pageInstanceGenerations.delete(oldName);
  if (generation !== undefined) pageInstanceGenerations.set(newName, generation);
  if (dirty.has(newName)) scheduleSave();
}
async function settleSavedTitleIdentity(name: string, id: string, dto: PageDto, rev: string): Promise<void> {
  if (dto.kind !== "page") return;
  const binding = captureBinding();
  const generation = pageInstanceGeneration(name);
  const title = pagePropertyEntries(dto.pre_block, dto.format === "org" ? "org" : "md")
    .find((entry) => entry.key.toLowerCase() === "title")?.value;
  let effective = title;
  if (!effective && titleIdentityIntents.has(name)) {
    try {
      const result = await readOwned(graphOwner(), backend().getPageByPath(id));
      if (result.kind === "stale") return;
      effective = result.value?.name;
    } catch (error) {
      pushToast(`Saved the page, but could not refresh its title: ${String(error)}`, "error");
      return;
    }
  }
  if (!stillBound(binding) || generation !== pageInstanceGeneration(name) || pageByName(name)?.id !== id) return;
  if (effective && effective !== name) {
    if (!rekeyPageIdentityByPath(id, effective, rev, true)) {
      pushToast("Saved the title, but its page identity could not be adopted safely. Reopen this page by its file path.", "error");
      return;
    }
  }
  titleIdentityIntents.delete(name);
}
/** Tombstone a page so any pending/in-flight save can't recreate its file. */
export function tombstone(name: string) {
  deletedPages.add(name);
}
/** Lift a delete tombstone (page re-created, or the delete failed). */
export function untombstone(name: string) {
  deletedPages.delete(name);
}
/** Drop a page's dirty + baseline state — its content is leaving the working set. */
export function forgetSaveState(name: string) {
  dirty.delete(name);
  kindLedger.delete(name);
  baseRev.delete(name);
  forgetSaveFailure(name);
  titleIdentityIntents.delete(name);
}
/** After flushAll has drained before a graph switch, cancel timers, invalidate
 *  in-flight saves (bump the graph token), and clear all guard state. */
export function resetSaveState() {
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  saveBurstStart = null;
  if (dataRevTimer) {
    clearTimeout(dataRevTimer);
    dataRevTimer = null;
  }
  graphToken++;
  dirty.clear();
  kindLedger.clear();
  titleIdentityIntents.clear();
  baseRev.clear();
  deletedPages.clear();
  landedAliasDrafts.clear();
  deletingGroupMembers.clear();
  for (const g of new Set([...groupOf.values(), ...sealedGroups])) {
    g.cancelled = true;
    for (const resolve of g.waiters) resolve(false);
    g.waiters.length = 0;
  }
  groupOf.clear();
  sealedGroups.clear();
  saveAttempts.clear();
  for (const name of [...lastSaveFailure.keys()]) forgetSaveFailure(name);
  setConflictReasons({});
}

// ---------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------

// Debounced query-recompute trigger: bump dataRev only after edits go quiet, so
// sustained typing doesn't re-run every visible query every save batch.
function scheduleDataRev() {
  if (dataRevTimer) clearTimeout(dataRevTimer);
  dataRevTimer = setTimeout(() => {
    dataRevTimer = null;
    bumpDataRev();
  }, 700);
}

function cutSourceMatches(expected: ClipboardSourcePage): boolean {
  const page = pageByName(expected.name);
  return !!page
    && page.name === expected.name
    && page.kind === expected.kind
    // A grant taken before the page's first save has no file id; that save
    // then records the id it created (`setPageId`) on the same instance, so
    // the generation check alone pins it.
    && (expected.path === undefined || page.id === expected.path)
    && pageInstanceGeneration(expected.name) === expected.generation;
}

function cutSourceUsable(expected: ClipboardSourcePage): boolean {
  return cutSourceMatches(expected)
    && !deletedPages.has(expected.name)
    && !isConflicted(expected.name);
}

function enqueueSave(
  name: string,
  force = false,
  expectedCutSource?: ClipboardSourcePage,
  decision?: ConflictReason,
): Promise<boolean> {
  const currentGroup = group(name);
  if (currentGroup?.state === "open") return enqueueGroup(currentGroup);
  const binding = captureBinding();
  const token = graphToken;
  const generation = pageInstanceGeneration(name);
  const prev = saveChain.get(name) ?? Promise.resolve(true);
  const next: Promise<SaveResult> = prev.then(
    () => doSave(name, force, binding, token, generation, currentGroup, expectedCutSource, decision),
    () => doSave(name, force, binding, token, generation, currentGroup, expectedCutSource, decision),
  );
  saveChain.set(name, next);
  void next.finally(() => {
    if (saveChain.get(name) === next) saveChain.delete(name);
  });
  return next.then((result) => {
    if (result !== "deferred") return result;
    const deferredGroup = group(name);
    return deferredGroup ? enqueueGroup(deferredGroup) : false;
  });
}

/** Write the page's CURRENT state once. No-op success if it isn't dirty and not
 *  forced. Sends `baseRev` (the version the editor loaded) so the backend
 *  conflicts against external changes; updates the baseline on success. On a
 *  conflict marks it (no clobber); on a transient error keeps it dirty + toasts. */
async function doSave(
  name: string,
  force: boolean,
  binding: Binding,
  token: number,
  generation: number | null,
  groupAtEnqueue: SaveGroup | undefined,
  expectedCutSource?: ClipboardSourcePage,
  decision?: ConflictReason,
): Promise<SaveResult> {
  if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
  // A cut-retirement save is authority-bound to the exact loaded page instance.
  // Check when this queued operation actually reaches its snapshot boundary, not
  // only when the caller enqueues it: another save may have been ahead of it.
  if (expectedCutSource && !cutSourceUsable(expectedCutSource)) return false;
  const currentGroup = group(name);
  if (currentGroup && (currentGroup !== groupAtEnqueue || currentGroup.state === "open")) return "deferred";
  if (deletedPages.has(name)) return true; // tombstoned — never recreate a deleted page
  if (!force && !dirty.has(name)) return true; // already saved by a prior link
  if (isConflicted(name) && !force) return false;
  if (force && (!decision || conflictReason(name) !== decision)) return false;
  const dto = pageToDto(name);
  if (!dto) return false;
  if (dto.guide) {
    console.warn("Refusing to persist ephemeral bundled Guide page");
    dirty.delete(name);
    return true;
  }
  if (dto.read_only) {
    console.error("Refusing to persist read-only page");
    dirty.delete(name);
    return false;
  }
  noteSaveAttempt(name);
  const baseline = decision?.observedRev !== undefined ? decision.observedRev : baseRev.get(name) ?? null;
  const kinds = pendingKinds(name, baseline === null);
  dirty.delete(name);
  kindLedger.delete(name);
  try {
    let id = pageByName(name)?.id;
    if (!id) {
      const resolved = await backend().resolvePage(dto.name, dto.kind);
      if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
      if (resolved.kind === "alias") {
        // A pathless draft may acquire an alias while it is open. Its blocks
        // belong to the alias owner, but the owner's existing bytes must win
        // the front of the page. Read its current revision and use an ordinary
        // guarded save; forceSave must not clobber an externally edited owner.
        const owner = resolved.owners[0] && await backend().getPageByPath(resolved.owners[0]);
        if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
        if (!owner || owner.read_only || owner.guide || isDirty(owner.name) || isSaving(owner.name) || isConflicted(owner.name)) {
          throw new Error("conflict");
        }
        const ownerGeneration = pageInstanceGeneration(owner.name);
        // A draft's property-only first root is folded into pre_block by
        // pageToDto. Keep those bytes too: on the owner they are ordinary
        // appended content, never a replacement for the owner's preamble.
        // A copy that already landed is replaced in place (L13), bound to this
        // exact draft instance; a refusal is an ordinary disk conflict.
        const appended = aliasOwnerPage(name, generation, owner, dto);
        if (!appended) throw new Error("conflict");
        const ownerRev = await saveOnePage(backend(), { id: owner.id, page: appended, baseRev: owner.rev ?? null, force: false,
          kinds: ["insert-blocks", "delete-page"] }, binding.backendGeneration);
        landedAliasDrafts.set(name, { owner: owner.id, blocks: aliasDraftBlocks(dto), generation });
        if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
        if (dirty.has(name) || isDirty(owner.name) || isSaving(owner.name)
            || isConflicted(owner.name) || pageInstanceGeneration(owner.name) !== ownerGeneration) {
          // The saved snapshot is durable, but a later edit must remain visible
          // in the draft rather than being discarded by the route change.
          throw new Error("conflict");
        }
        const landed = { ...appended, rev: ownerRev };
        landedAliasDrafts.delete(name);
        forgetPage(name);
        reloadPage(landed);
        loadSingle(landed);
        aliasDraftRouteHandler?.(owner.name, owner.kind);
        pushToast(`Moved “${name}” into its alias owner “${owner.name}”.`, "info");
        bumpPageInventoryRev();
        forgetSaveFailure(name);
        return true;
      }
      id = resolved.id;
    }
    const rev = await saveOnePage(backend(), { id, page: dto, baseRev: baseline, force: false,
      kinds }, binding.backendGeneration);
    // A reload/rename/delete/rebind while savePages was in flight invalidates the
    // retirement proof even if those bytes landed. Never let that stale success
    // authorize identity reuse or update the replacement instance's baseline.
    if (expectedCutSource && !cutSourceUsable(expectedCutSource)) return false;
    if (token === graphToken && stillBound(binding) && pageInstanceGeneration(name) === generation) {
      // Record the file this save wrote, but only on the instance that asked:
      // a reload/rebind meanwhile carries its own id.
      setPageId(name, id);
      baseRev.set(name, rev);
      if (dto.pre_block) adoptFoldedPageHeader(name, dto.pre_block);
      await settleSavedTitleIdentity(name, id, dto, rev);
      if (baseline === null) bumpPageInventoryRev();
      forgetSaveFailure(name);
      return true;
    }
    return false;
  } catch (e) {
    if (token === graphToken && stillBound(binding) && pageInstanceGeneration(name) === generation) {
      restoreKinds(name, kinds);
      const family = errorFamily(e);
      if (family === "conflict" || family === "deleted" || family === "twin"
          || family === "read-only" || family === "invalid-target") {
        const observedRev = (e as { diskRev?: string | null }).diskRev;
        markConflict(name, { kind: "disk-changed" }, family === "deleted" ? null : observedRev);
      } else {
        dirty.add(name); // keep pending — retried on next edit / flush
      }
      if (family !== "conflict")
        reportSaveFailure(name, family, `Couldn't save “${name}” — ${family === "deleted" ? "the file was deleted on disk; your edits remain in the editor" : String(e)}${describeSavePlatformStep((e as { platformStep?: SavePlatformStep | null }).platformStep ?? null)}`);
    }
    return false;
  }
}

export function scheduleSave() {
  if (!doc.loaded) return;
  const now = Date.now();
  saveBurstStart ??= now;
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    saveTimer = null;
    saveBurstStart = null;
    const names = [...dirty];
    void (async () => {
      const results = await Promise.all(names.map((n) => enqueueSave(n)));
      // The backend cache now reflects these edits → let queries recompute, but
      // coalesce: re-running every on-screen query is a whole-graph scan, so wait
      // for a lull instead of firing on every 400ms save batch.
      if (results.some(Boolean)) scheduleDataRev();
    })();
  }, Math.min(400, Math.max(0, 3000 - (now - saveBurstStart))));
}

/** Save one page immediately, bypassing the debounce — for actions that must
 *  durably persist before the user might quit (e.g. parking a block in the
 *  sidebar writes an id:: that has to survive a restart). Returns success. */
export async function flushPage(name: string): Promise<boolean> {
  if (!doc.loaded) return false;
  const ok = await enqueueSave(name);
  if (ok && dirty.size === 0 && saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
    saveBurstStart = null;
  }
  if (ok) scheduleDataRev();
  return ok;
}

/** Retire every page touched by a cut against the exact instances recorded in
 * the clipboard grant. Preflight the whole set before starting any writes, then
 * bind the same identity+generation check into each queued save at snapshot and
 * completion. A clean page still queues a checked no-op so an earlier save is
 * drained before it counts as retired. */
export async function flushCutSourcePages(sources: readonly ClipboardSourcePage[]): Promise<boolean> {
  if (
    !doc.loaded
    || sources.length === 0
    || new Set(sources.map((source) => source.name)).size !== sources.length
    || !sources.every(cutSourceUsable)
  ) return false;
  const results = await Promise.all(
    sources.map((source) => enqueueSave(source.name, false, source)),
  );
  if (results.some(Boolean)) scheduleDataRev();
  return results.every(Boolean);
}

/** Final synchronous retirement guard used immediately before identity insert. */
export function cutSourcePagesRetired(sources: readonly ClipboardSourcePage[]): boolean {
  return sources.length > 0 && sources.every((source) =>
    cutSourceMatches(source)
    && !dirty.has(source.name)
    && !saveChain.has(source.name)
    && !deletedPages.has(source.name)
    && !group(source.name)
    && !isConflicted(source.name)
  );
}

/** Persist every dirty page now and wait for them (incl. anything mid-write) —
 *  for graph switch / restore / app close. Returns true only if everything
 *  landed (no conflicts or errors), so the caller can abort a destructive
 *  transition rather than discard the un-saved edit. */
export async function flushAll(): Promise<boolean> {
  if (saveTimer) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }
  saveBurstStart = null;
  let landed = false;
  // Drain repeatedly: an edit made WHILE a save is in flight re-dirties the page,
  // and a queued save may still be running, so one pass can miss work. Keep
  // flushing until nothing is pending (bounded against a persistently-failing
  // save).
  for (let i = 0; i < 4; i++) {
    const names = new Set<string>([...dirty, ...saveChain.keys()]);
    const assetWrites = [...assetWriteChain];
    if (names.size === 0 && assetWrites.length === 0) break;
    const [results] = await Promise.all([
      Promise.all([...names].map((n) => enqueueSave(n))),
      Promise.all(assetWrites),
    ]);
    if (results.some(Boolean)) landed = true;
  }
  if (landed) bumpDataRev();
  // Success only if nothing is still pending AND there are no unresolved
  // conflicts (a conflicted page's edit is NOT on disk) — so a destructive
  // transition (graph switch / restore / close) can abort instead of discarding it.
  return dirty.size === 0 && saveChain.size === 0 && assetWriteChain.size === 0 && conflicts().length === 0;
}

/** Resolve a save conflict by overwriting the on-disk file with the in-memory
 *  version ("keep mine"). Returns whether the overwrite succeeded — the caller
 *  must not clear the conflict unless it did. */
export async function forceSave(name: string): Promise<boolean> {
  const g = group(name);
  if (g) {
    const reason = conflictReason(name);
    if (reason) g.forced.set(name, reason);
    noteKinds(name, "replace-page");
    return enqueueGroup(g);
  }
  const decision = conflictReason(name);
  if (!decision) return false;
  dirty.add(name); // ensure doSave writes even though it's parked as conflicted
  noteKinds(name, "replace-page");
  const ok = await enqueueSave(name, true, undefined, decision);
  if (ok && conflictReason(name) === decision) clearConflict(name);
  if (!ok) pushToast(`Couldn't overwrite “${name}”.`, "error");
  return ok;
}

/** Apply one conflict-bar decision with its binding and group consequences. */
export async function resolveConflict(name: string, choice: "mine" | "disk"): Promise<boolean> {
  if (!isConflicted(name)) return false;
  if (choice === "mine") {
    if (conflictReason(name)?.kind === "repeated") return false;
    const g = group(name);
    if (g?.state === "sealed") {
      const binding = captureBinding(), token = graphToken, generation = pageInstanceGeneration(name);
      await g.request;
      if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
      return isConflicted(name) ? resolveConflict(name, "mine") : true;
    }
    if (g) {
      g.forced.set(name, conflictReason(name)!);
      noteKinds(name, "replace-page");
      changedGroups();
      if ([...g.members].some((member) => isConflicted(member) && !decidedConflict(g, member))) return true;
    }
    return forceSave(name);
  }
  const binding = captureBinding(), token = graphToken, generation = pageInstanceGeneration(name);
  const page = pageByName(name);
  const kind = page?.kind ?? "page";
  const id = page?.id;
  await Promise.all([...sealedGroups].filter((g) => g.members.has(name)).map((g) => g.request));
  if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
  const currentGroup = group(name);
  if (currentGroup?.state === "sealed") return false;
  if (currentGroup?.forced.delete(name)) changedGroups();
  const reason = conflictReason(name);
  const attempt = saveAttempts.get(name) ?? 0;
  let dto: Awaited<ReturnType<ReturnType<typeof backend>["getPage"]>>;
  try { dto = id ? await backend().getPageByPath(id) : await backend().getPage(name, kind); }
  catch (error) {
    if (stillBound(binding) && token === graphToken && pageInstanceGeneration(name) === generation)
      pushToast(`Couldn't read “${name}” from disk — ${String(error)}`, "error");
    return false;
  }
  if (!stillBound(binding) || token !== graphToken || pageInstanceGeneration(name) !== generation) return false;
  if (group(name) !== currentGroup || group(name)?.state === "sealed"
      || conflictReason(name) !== reason
      || currentGroup?.forced.has(name) || (saveAttempts.get(name) ?? 0) !== attempt) return false;
  releaseGroup(name);
  if (dto) reloadPage(dto);
  else forgetPage(name);
  dirty.delete(name);
  kindLedger.delete(name);
  clearConflict(name);
  return true;
}

// ---------------------------------------------------------------------------
// Concord live-draft conflicts (og 8e)
// ---------------------------------------------------------------------------

/** The live-conflict draft of `name`: its editor DTO, the revision it was
 *  edited from (selects the Concord-ledger base) and its exact loaded
 *  instance. Only a plain `disk-changed` conflict outside any save group has
 *  one; the group kinds (released / repeated / alias-owner-busy) stay on the
 *  conflict bar. O(page). */
export function liveConflictDraft(name: string): { page: PageDto; baseRev: string | null; generation: number } | null {
  const reason = conflictReasons()[name];
  if (reason?.kind !== "disk-changed" || group(name)) return null;
  const page = pageToDto(name), generation = pageInstanceGeneration(name);
  return page && generation !== null ? { page, baseRev: baseRev.get(name) ?? null, generation } : null;
}

/** True when two drafts carry the same editable content and identity (the
 *  backend-populated revision is ignored). */
export function sameLiveDraft(a: PageDto, b: PageDto): boolean {
  const key = (p: PageDto) => JSON.stringify([p.name, p.kind, p.title, p.pre_block, p.blocks, p.format ?? "md"]);
  return key(a) === key(b);
}

/** Install a live resolution the backend committed (master 7e1b6ec42 /
 *  ba80a151e, on og's replacement gate): drain this page's save chain, then,
 *  in one synchronous step, replace the loaded instance with `resolved` and
 *  clear its dirty, kinds and conflict state; `resolved.rev` becomes the save
 *  baseline, so nothing pre-merge can autosave over the result. Only when the
 *  instance is the one reviewed (`generation`), its draft is still `reviewed`
 *  and no block of it is being edited. Otherwise the newer draft is kept and
 *  stays conflicted against the resolved revision: the user reviews again.
 *  Returns what happened. O(page). */
export async function installLiveResolution(name: string, generation: number, reviewed: PageDto,
  resolved: PageDto & { id?: string }): Promise<"installed" | "kept" | "gone"> {
  const binding = captureBinding(), token = graphToken;
  const tail = saveChain.get(name);
  if (tail) await tail.catch(() => false);
  if (!stillBound(binding) || token !== graphToken) return "gone";
  if (pageInstanceGeneration(name) !== generation) return "gone";
  const now = pageToDto(name);
  const ed = editingId();
  if (!now || !sameLiveDraft(now, reviewed) || saveChain.has(name) || (ed && doc.byId[ed]?.page === name)) {
    markConflict(name, { kind: "disk-changed" }, resolved.rev ?? null);
    return "kept";
  }
  reloadPage(resolved);
  dirty.delete(name);
  kindLedger.delete(name);
  forgetSaveFailure(name);
  clearConflict(name);
  return "installed";
}
