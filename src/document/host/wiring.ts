// The window's page persistence (STEP3-DESIGN §4; plan v3 P2b): one `HostClient`
// per graph binding over the `page_*` commands, the document port over the
// working set, and the facade the document module calls (`markDirty`,
// `flushPage`, `createPage`, conflicts, transfers, deletion). This is the only
// module that issues `page_*` commands (guard: boundary.guard.test.ts); every
// page write is the host's (the audited save path).

import { createEffect, createRoot, createSignal, on, untrack } from "solid-js";
import { backend } from "../../backend";
import { benchEvent } from "../../benchEvents";
import { captureBinding, clearOnBindingInvalidated } from "../../binding";
import type { ClipboardSourcePage } from "../../clipboard";
import { conflictPolicyAlwaysAsk, holdExternalChange } from "../../conflictPolicy";
import type { EditKind, EditKinds } from "../../editKind";
import { editingId } from "../../editorController";
import { pagePropertyEntries } from "../../editor/properties";
import { errorFamily } from "../../errorFamily";
import { bindingOwner, ownedWhen, readOwned, readOwnedResource, writeOwned } from "../../owned";
import { bumpDataRev, bumpPageInventoryRev } from "../../graphSession";
import { dismissToast, pushToast } from "../../toasts";
import { msg, renderEnglish, type UiText } from "../../i18n";
import type { BlockDto, MergeDecision, PageDto, PageKind } from "../../types";
import { openUnsavedRecovery } from "../../unsavedRecovery";
import { assetWritesStarted, pendingAssetWrites } from "../assetWrites";
import { aliasDraftBlocks, appendAliasDraft, emptyPage, pageToDto, replaceLandedAliasDraft } from "../convert";
import { isBlockMoving } from "../edits/moves";
import { adoptFoldedPageHeader } from "../edits/properties";
import { graphRewriteFrozen } from "../graphRewriteState";
import { pageInstanceGeneration, rekeyPageInstance } from "../instance";
import { doc, pageByName, setPageId } from "../model";
import { draftPinned, ensurePageLoaded, forgetPage, installPageContent, rekeyPageIdentityByPath, reportPageLoadRefusal } from "../workingSet";
import { HostClient, reviewedDiskToken, type BaselineRev, type DocumentPort, type HostCutSource } from "./client";
import { runSequence, type SequenceStep } from "./planner";
import type { DiskToken, HostPort, MailNotice, PageMail, PageOperation, PageRefusal } from "./protocol";
import { settle, unpublishedAfterDrain } from "./settle";
import { beginResultsChange, installResultsPendingGate, resetResultsEpoch } from "../../queryResultsEpoch";

// ------------------------------------------------------------------ state

let client: HostClient | null = null;
/** The client answered `bind`: commands carry its session. */
let bound = false;
/** Edits made before the binding's session is known, replayed at bind. */
const early = new Map<string, EditKind[]>();
/** The file revision each loaded page's text was installed from (Concord's
 * ledger base, and the document port's baseline facts). */
const baseRev = new Map<string, string | null>();
/** Pages deleted in this window: no edit re-opens them until they load again. */
const deletedPages = new Set<string>();
/** Transfer endpoints (§8): frozen, with the host's text held apart from the display. */
const transferPages = new Set<string>();
const speculation = new Map<string, PageDto>();
const titleIntents = new Set<string>();
/** Alias drafts whose copy landed in their owner while a later edit kept the
 * draft open (L13): a retry replaces that copy, never appends again. */
const landedAliasDrafts = new Map<string, { owner: string; blocks: BlockDto[]; generation: number | null }>();
const landing = new Set<string>();
let inventoryPending = false;

const [conflictList, setConflictList] = createSignal<string[]>([]);
/** Moves whenever a page's busy, conflicted, open or frozen state changes. */
const [hostRevision, setHostRevision] = createSignal(0);
let stateKey = "";
installResultsPendingGate(() => {
  hostRevision();
  return pendingDataRevision() || early.size > 0
    || !!client?.names().some(name => client!.busy(name) || client!.owes(name));
}, "own-work");
installResultsPendingGate(() => {
  hostRevision();
  return transferPages.size > 0
    || !!client?.names().some(name => client!.conflicted(name) || client!.holdsContent(name));
});

function host(): HostClient | null {
  return bound ? client : null;
}

// ------------------------------------------------------------------ ports

const failed = (error: unknown): PageRefusal => ({ reason: "failed", message: String(error) });

/** `page_save_now` only moves a due save earlier: if it fails the host still
 * saves on its own cadence, and the barrier's next `page_wait` on the same
 * bridge reports the failure as a failed barrier. */
function ignoreSaveHintFailure(_error: unknown): void {}

// A lost flush answer is harmless: the owner stops waiting and refuses the
// command-line write as `page-busy`; nothing is written.
function ignoreLostFlushAnswer(_error: unknown): void {}

function hostPort(): HostPort {
  const b = backend();
  return {
    windowReloaded: () => b.pageWindowReloaded(),
    open: async (session, id, page) => {
      const reply = await b.pageOpen(session, id, page).catch(failed);
      if ("reason" in reply && reply.reason === "alias") queueAliasLanding(page.name, reply.owners);
      return reply;
    },
    submit: async (session, id, key, dto, version, resolve, kinds) => {
      const refusal = await b.pageSubmit(session, id, key, dto, version, resolve, kinds).catch(failed);
      if (!refusal) admittedText(key, dto);
      return refusal;
    },
    move: (session, id, source, receiver, kinds) => b.pageMove(session, id, source, receiver, kinds).catch(failed),
    discard: (session, id, key) => b.pageDiscard(session, id, key).catch(failed),
    close: (session, id, key) => b.pageClose(session, id, key).catch(failed),
    saveNow: (session, keys) => b.pageSaveNow(session, keys).catch(ignoreSaveHintFailure),
    // One bounded wait (F1); a failed call is a failed barrier, never success.
    waitPublished: (session, needs) => b.pageWait(session, needs, 5000).catch(() => false),
    // No answer (another session, a failed read) is unknown debt, never "none".
    owed: (session, paths) => b.pageOwed(session, paths).then((owed) => owed ?? false).catch(() => false as const),
  };
}

const documentPort: DocumentPort = {
  facts(name) {
    const page = pageByName(name);
    const instance = page ? pageInstanceGeneration(name) : null;
    if (!page || instance === null) return null;
    const known = baseRev.get(name);
    const rev: BaselineRev = !page.id || known === null ? { kind: "no-file" }
      : known === undefined ? { kind: "unknown" } : { kind: "file", rev: known };
    return { instance, path: page.id ?? null, kind: page.kind, format: page.format ?? "md", rev };
  },
  dto: (name) => speculation.get(name) ?? pageToDto(name),
  install(name, content) {
    const dto = content.kind === "page" ? content.dto : null;
    if (transferPages.has(name)) {
      const page = pageByName(name);
      speculation.set(name, dto ?? { ...emptyPage(name, page?.kind ?? "page"), rev: null });
      return;
    }
    installPageContent(name, dto, client?.keyOf(name));
  },
  holds: (name) => isBlockMoving() || draftPinned(name),
  holdsPush(name) {
    const page = pageByName(name);
    if (!page || !conflictPolicyAlwaysAsk()) return false;
    holdExternalChange(name, { name, kind: page.kind, path: page.id, created: false, removed: false });
    return true;
  },
  tombstoned: (name) => deletedPages.has(name),
  took: (name, rev) => { baseRev.set(name, rev); },
};

/** The document port for one binding's client: a client the binding has
 * replaced (a graph switch, a restore) changes nothing in this window's
 * document, so a late Open answer cannot install the old graph's text into a
 * same-named page of the new one (I-20). */
function portFor(current: () => boolean): DocumentPort {
  return {
    ...documentPort,
    install: (name, content) => { if (current()) documentPort.install(name, content); },
    holdsPush: (name) => current() && documentPort.holdsPush(name),
    took: (name, rev) => { if (current()) documentPort.took(name, rev); },
  };
}

/** The host took a text: the page now has its file, and a folded page header is
 * the page's header (what a save used to adopt). */
function admittedText(key: string, dto: PageDto): void {
  const name = client?.nameOfKey(key) ?? dto.name;
  const page = pageByName(name);
  if (!page) return;
  if (!page.id) {
    setPageId(name, key);
    inventoryPending = true;
  }
  if (dto.pre_block) adoptFoldedPageHeader(name, dto.pre_block);
  queueMicrotask(() => void settleTitleIdentity(name, key, dto));
}

// ------------------------------------------------------------------ lifecycle

/** Bind this window to the graph's page host (after a graph load). A second
 * call for the same binding (a restore relaunched the host) rebinds. */
export async function bindHost(): Promise<void> {
  listen();
  watch();
  if (client && bound) {
    await client.rebind();
    hostChanged();
    return;
  }
  const next: HostClient = new HostClient(hostPort(), portFor(() => client === next),
    { pending: pendingAssetWrites, started: assetWritesStarted });
  next.onChange = hostChanged;
  client = next;
  bound = false;
  try {
    await next.bind();
  } catch (error) {
    if (client === next) pushToast(msg("document-wiring-tine-can-t-save-this-graph", { value1: String(error) }), "error");
    return;
  }
  if (client !== next) return;
  bound = true;
  for (const [name, kinds] of early) if (pageByName(name) && kinds.length) next.noteEdit(name, [kinds[0], ...kinds.slice(1)]);
  early.clear();
  hostChanged();
}

// The binding's page client and its baseline/tombstone state end with the
// binding (store reset: graph switch, restore); the caller settled or reported
// its input first.
clearOnBindingInvalidated(() => {
  resetResultsEpoch();
  // Its outstanding requests settle unanswered ("rebound"), so no caller of
  // the old graph waits forever (I-20).
  const old = client;
  client = null;
  old?.retire();
  bound = false;
  early.clear();
  baseRev.clear();
  deletedPages.clear();
  transferPages.clear();
  speculation.clear();
  titleIntents.clear();
  landedAliasDrafts.clear();
  landing.clear();
  problemToasts.clear();
  inventoryPending = false;
  if (dataTimer) clearTimeout(dataTimer);
  dataTimer = null;
  pendingNeeds.clear();
  hostChanged();
});

/** The last host answer this window consumed (a restore's watermark, S8). */
export function consumedAnswer(): number {
  return host()?.consumed ?? 0;
}

let listening = false;
function listen(): void {
  if (listening) return;
  listening = true;
  void backend().onPageMail((mail: PageMail) => {
    const c = client;
    if (!c) return;
    const finish = mail.page && mail.page.text.kind !== "unchanged" && !mail.answer ? beginResultsChange() : () => {};
    try { c.receive(mail); } finally { finish(); }
    const name = mail.session === c.session ? c.nameOfKey(mail.key) : null;
    if (!name || !mail.answer?.took) return;
    notePublication(mail.key, mail.answer.version);
  });
}

let watching = false;
function watch(): void {
  if (watching) return;
  watching = true;
  createRoot(() => {
    // B-Q2: a block entering editing opens its page; leaving releases it.
    let held: { name: string; client: HostClient; release: () => void } | null = null;
    createEffect(() => {
      hostRevision();
      const ed = editingId();
      const name = ed ? doc.byId[ed]?.page ?? null : null;
      untrack(() => {
        const c = host();
        if (held && held.name === name && held.client === c) return;
        held?.release();
        held = null;
        if (name && c && pageWritable(name)) held = { name, client: c, release: c.acquire(name) };
      });
    });
    createEffect(on(() => isBlockMoving(), (moving) => { if (!moving) releaseHolds(); }, { defer: true }));
  });
}

/** A content hold ended (a move settled, a draft pin released): install the
 * host content it held back where nothing newer exists. */
export function releaseHolds(): void {
  const c = host();
  for (const name of c?.names() ?? []) c!.release(name);
}

function pageWritable(name: string): boolean {
  const page = pageByName(name);
  return !graphRewriteFrozen() && !!page && !page.readOnly && !page.guide;
}

// ------------------------------------------------------------------ data revision

const pendingNeeds = new Map<string, number>();
let dataTimer: ReturnType<typeof setTimeout> | null = null;
let dataWaits = 0;

/** Whole-graph views recompute once the host published what it took, after
 * edits go quiet (today's 700 ms coalescing). */
function notePublication(key: string, version: number): void {
  pendingNeeds.set(key, Math.max(version, pendingNeeds.get(key) ?? 0));
  setHostRevision(value => value + 1);
  if (dataTimer) clearTimeout(dataTimer);
  dataTimer = setTimeout(() => {
    dataTimer = null;
    const c = client;
    if (!c) return;
    const needs = [...pendingNeeds].map(([need, at]) => ({ key: need, version: at }));
    pendingNeeds.clear();
    dataWaits += 1;
    void c.published(needs).finally(() => {
      dataWaits -= 1;
      if (client !== c) return;
      bumpDataRev();
      setHostRevision(value => value + 1);
      if (inventoryPending) { inventoryPending = false; bumpPageInventoryRev(); }
    });
  }, 700);
}

/** Read-only: derived membership waits for the coalesced publication revision. */
export function pendingDataRevision(): boolean {
  return dataTimer !== null || dataWaits > 0;
}

// ------------------------------------------------------------------ state view

export type ConflictReason = { kind: "disk-changed"; observedRev?: string | null };
export type UnsavedState = "Saving" | "Conflict" | "Not saved";

/** Pages the conflict UI shows: conflicted, once the host reported the
 * conflict's draft custody (§3.3). */
export const conflicts = conflictList;

export function hostState(): number {
  return hostRevision();
}

function hostChanged(): void {
  const c = host();
  const names = c ? c.names() : [];
  const shown = names.filter((name) => c!.conflicted(name) && c!.notice(name)?.conflictReported);
  const current = conflictList();
  if (shown.length !== current.length || shown.some((name, i) => name !== current[i])) setConflictList(shown);
  const key = names.map((name) => `${name}\u0000${+c!.busy(name)}${+c!.owes(name)}${+c!.conflicted(name)}${+c!.isOpen(name)}${+c!.holdsContent(name)}`)
    .join("\u0001")
    + `\u0002${[...early.keys()].join("\u0001")}`;
  if (key !== stateKey) {
    stateKey = key;
    setHostRevision((n) => n + 1);
  }
  reportProblems(c, names);
}

export function isDirty(name: string): boolean {
  hostRevision();
  return early.has(name) || !!host()?.isDirty(name);
}

/** Sent and not yet answered. */
export function isSaving(name: string): boolean {
  hostRevision();
  const c = host();
  return !!c && c.busy(name) && !c.isDirty(name);
}

export function isConflicted(name: string): boolean {
  hostRevision();
  return !!host()?.conflicted(name);
}

export function conflictReason(name: string): ConflictReason | undefined {
  if (!isConflicted(name)) return undefined;
  const disk = host()?.disk(name);
  return { kind: "disk-changed", observedRev: disk ? (disk.kind === "no-file" ? null : disk.rev) : undefined };
}

/** Pages whose input the host has not answered, has not published (the
 * client's publication debt, J6) or cannot save: the working set never evicts them. */
export function unsettledPages(): string[] {
  const c = host();
  return [...early.keys(), ...transferPages,
    ...(c ? c.names().filter((name) => c.busy(name) || c.owes(name) || c.conflicted(name)) : [])];
}

/** The page's client state blocks a reload: input unanswered or unpublished, or a conflict. */
export function hostBlocksReload(name: string): boolean {
  const c = host();
  return early.has(name) || transferPages.has(name) || (!!c && (c.busy(name) || c.owes(name) || c.conflicted(name)));
}

/** The host holds the page for this window: its mail is the page's authority. */
export function hostHolds(name: string): boolean {
  const c = host();
  return !!c && (c.isOpen(name) || c.busy(name) || c.conflicted(name));
}

/** "Reload from disk" for a held external change: install what the host holds. */
export function acceptHeldPush(name: string): void {
  host()?.release(name, true);
}

/** Every page whose edits are not on disk yet, with its loaded text (GH #540
 * recovery panel and close prompt): unanswered, unpublished (J6), conflicted
 * or failing. O(open pages) plus one DTO per entry. */
export function unsavedDrafts(): { name: string; state: UnsavedState; path: string | null; page: PageDto | null }[] {
  const c = host();
  const names = new Set([...early.keys(),
    ...(c ? c.names().filter((name) => c.busy(name) || c.owes(name) || c.conflicted(name) || c.notice(name)?.saveError) : [])]);
  return [...names].map((name) => ({
    name,
    state: c?.conflicted(name) ? "Conflict" as const
      : c && (c.busy(name) || c.owes(name)) && !c.isDirty(name) && !c.notice(name)?.saveError ? "Saving" as const
        : "Not saved" as const,
    path: pageByName(name)?.id ?? null,
    page: pageToDto(name),
  }));
}

export function unsavedPageCount(): number {
  return unsavedDrafts().length;
}

// ------------------------------------------------------------------ problems

const problemToasts = new Map<string, { said: string; id: number }>();
const SILENT = new Set(["not-admitted", "alias", "stale", "not-held", "not-open", "endpoint-busy", "rebound"]);

/** R-CREATE-UNREADABLE-OWNER (docs/storage-contract.md): the host refused to
 * create `name` because a file it cannot read may already be that page. */
function unreadableOwnerMessage(name: string, owner: string | undefined) {
  return msg("document-create-unreadable", { name, owner: owner ?? "", hasOwner: owner == null ? 0 : 1 });
}

function refusalMessage(name: string, refusal: PageRefusal | string): import("../../i18n").Msg | null {
  if (typeof refusal === "string") {
    if (refusal === "read-failed") return msg("document-read-refused", { name });
    if (refusal === "draft-failed") return msg("document-draft-refused", { name });
    return null;
  }
  switch (refusal.reason) {
    case "invalid-target":
    case "read-only":
    case "failed":
      return msg("document-save-refused", { name, detail: refusal.message });
    case "twin":
      return msg("document-save-twin", { name, existing: refusal.existing });
    case "undecodable":
      return msg("document-save-undecodable", { name });
    case "unreadable-owner":
      return unreadableOwnerMessage(name, refusal.file);
    default:
      return null;
  }
}

/** ` (<platform step>, os error <n>)` for a failed save's toast, when the
 * host named them (GH #538, #590: `io:InvalidInput` alone could not tell an
 * Android no-replace-rename refusal from a failed temporary-file write). */
function saveStep(notice: MailNotice): string {
  const parts = [notice.operation, notice.osError === null ? null : `os error ${notice.osError}`]
    .filter((part): part is string => !!part);
  return parts.length ? ` (${parts.join(", ")})` : "";
}

/** One sticky toast per page while its host refusal or notice stands (GH #540). */
function reportProblems(c: HostClient | null, names: string[]): void {
  const seen = new Set<string>();
  for (const name of names) {
    const refusal = c!.refusal(name);
    const notice = c!.notice(name);
    const text = (refusal && !(typeof refusal === "object" ? SILENT.has(refusal.reason) : SILENT.has(refusal))
      ? refusalMessage(name, refusal) : null)
      ?? (notice?.twin ? msg("document-save-twin", { name, existing: notice.twin }) : null)
      ?? (notice?.saveError ? msg("document-save-retrying", { name, step: saveStep(notice) }) : null)
      ?? (notice?.dropped ? msg("document-operation-unrecorded", { name }) : null)
      ?? (notice?.draftError ? msg("document-draft-unwritten", { name }) : null);
    if (!text) continue;
    seen.add(name);
    const shown = problemToasts.get(name);
    const key = JSON.stringify(text);
    if (shown?.said === key) continue;
    if (shown) dismissToast(shown.id);
    problemToasts.set(name, { said: key, id: pushToast(text, "error",
      { sticky: true, action: { label: msg("document-review-unsaved"), run: openUnsavedRecovery } }) });
  }
  for (const [name, shown] of [...problemToasts]) {
    if (seen.has(name)) continue;
    dismissToast(shown.id);
    problemToasts.delete(name);
  }
}

// ------------------------------------------------------------------ edits

/** The dirty funnel: every programmatic page mutation, with its edit kinds. */
export function markDirty(name: string, kinds: EditKind | EditKinds): void {
  const page = pageByName(name);
  if (!page || page.readOnly || page.guide || deletedPages.has(name)) return;
  const c = host();
  if (!c) {
    const pending = early.get(name) ?? [];
    for (const kind of typeof kinds === "string" ? [kinds] : kinds) if (!pending.includes(kind)) pending.push(kind);
    early.set(name, pending);
    hostChanged();
    return;
  }
  c.noteEdit(name, kinds);
}

/** Only explicit edits to the page's own title read the file's effective name
 * once that title is removed; ordinary saves stay page-bounded. */
export function noteTitleIdentityIntent(name: string): void {
  titleIntents.add(name);
}

/** After the host took a text: a changed `title::` renames the loaded page
 * (navigation, working set, client) by its exact file. */
async function settleTitleIdentity(name: string, path: string, dto: PageDto): Promise<void> {
  if (dto.kind !== "page") return;
  const c = host();
  const generation = pageInstanceGeneration(name);
  let effective = pagePropertyEntries(dto.pre_block, dto.format === "org" ? "org" : "md")
    .find((entry) => entry.key.toLowerCase() === "title")?.value;
  if (!effective && titleIntents.has(name) && c) {
    if (!await settle(c, [name])) return;
    try {
      const read = await readOwned(bindingOwner(), backend().getPageByPath(path));
      if (read.kind === "stale") return;
      effective = read.value?.name;
    } catch (error) {
      pushToast(msg("document-wiring-saved-the-page-but-could-not-refresh-its-title", { value1: String(error) }), "error");
      return;
    }
  }
  if (host() !== c || pageInstanceGeneration(name) !== generation || pageByName(name)?.id !== path) return;
  if (effective && effective !== name && !rekeyPageIdentityByPath(path, effective, baseRev.get(name) ?? null, true)) {
    pushToast(msg("document-wiring-saved-the-title-but-its-page-identity-could-not-be"), "error");
    return;
  }
  titleIntents.delete(name);
}

/** A title-identity rename moved the loaded instance to `newName`. */
export function rekeyPage(oldName: string, newName: string, rev: string | null): void {
  client?.rekey(oldName, newName);
  rekeyPageInstance(oldName, newName);
  baseRev.delete(oldName);
  baseRev.set(newName, rev);
  const pending = early.get(oldName);
  early.delete(oldName);
  if (pending) early.set(newName, pending);
  if (titleIntents.delete(oldName)) titleIntents.add(newName);
}

/** The page's text was replaced from a file read (load or reload) at `rev`. */
export function setBaseRev(name: string, rev: string | null): void {
  baseRev.set(name, rev);
}

export function baseRevFor(name: string): string | null | undefined {
  return baseRev.get(name);
}

export function untombstone(name: string): void {
  deletedPages.delete(name);
}

/** The page's loaded text leaves the working set (eviction, forget). */
export function forgetSaveState(name: string): void {
  baseRev.delete(name);
  early.delete(name);
  titleIntents.delete(name);
  landedAliasDrafts.delete(name);
}

/** A multi-page transfer is running: undo and redo wait (they would replay
 * over text the host holds apart from the display). */
export function transferInProgress(): boolean {
  hostRevision();
  return transferPages.size > 0;
}

/** A page whose input is frozen for a transition (a transfer or a freeze). */
export function pageFrozen(name: string): boolean {
  hostRevision();
  return transferPages.has(name) || !!client?.isFrozen(name);
}

export function refuseConflictedMove(pages: Iterable<string>): boolean {
  const blocked = [...pages].find(isConflicted);
  if (!blocked) return false;
  pushToast(msg("document-wiring-resolve-the-conflict-on-first", { value1: blocked }), "error");
  return true;
}

// ------------------------------------------------------------------ barriers

/** Send the page's input and wait until the host published it (`settle`). */
export async function flushPage(name: string): Promise<boolean> {
  const c = host();
  if (!c) return !early.has(name);
  return settle(c, [name], { noConflict: true });
}

/** Every page's input and the tracked asset writes, published (switch, restore, print). */
export async function flushAll(): Promise<boolean> {
  const c = host();
  if (!c) return early.size === 0;
  return settle(c, "all", { assets: true, noConflict: true });
}

/** A block reference's target (§8, Q2): published bytes that contain `witness`. */
export async function settleWithWitness(name: string, witness: string): Promise<boolean> {
  const c = host();
  return !!c && settle(c, [name], { noConflict: true, witness });
}

/** Rename's drain (S6): every page whose input is not published after it. */
export async function unpublishedPages(): ReturnType<typeof unpublishedAfterDrain> {
  const c = host();
  if (!c) return [...early.keys()].map((name) => ({ key: null, name, state: "unsent" as const, conflict: false }));
  return unpublishedAfterDrain(c);
}

/** Stamp a cut source with its host page and this session (R7). */
export function stampCutSource(source: ClipboardSourcePage): ClipboardSourcePage {
  const c = host();
  return c ? c.stampCutSources([source])[0] : source;
}

/** Retire every page a cut touched, against the exact instances the grant names. */
export async function flushCutSourcePages(sources: readonly ClipboardSourcePage[]): Promise<boolean> {
  const c = host();
  const stamped = sources as readonly HostCutSource[];
  if (!c || !c.cutSourcesUsable(stamped)) return false;
  return await settle(c, stamped.map((source) => source.name)) && c.cutSourcesUsable(stamped);
}

/** Final synchronous retirement check immediately before identity insertion. */
export function cutSourcePagesRetired(sources: readonly ClipboardSourcePage[]): boolean {
  const c = host();
  return !!c && c.cutSourcesRetired(sources as readonly HostCutSource[]);
}

// ------------------------------------------------------------------ conflicts (§9)

/** Keep mine: send the text on the disk state the conflict showed. Take disk:
 * consume the unsent input and install the host's text. */
export async function resolveConflict(name: string, choice: "mine" | "disk"): Promise<boolean> {
  const c = host();
  if (!c?.conflicted(name)) return false;
  if (choice === "disk") return (await c.discard(name)).refusal === null;
  const disk = c.disk(name);
  const ticket = disk && c.reviewTicket(name, disk.kind === "no-file" ? "absent" : disk.rev);
  const dto = pageToDto(name);
  if (!ticket || !dto) return false;
  const answer = await c.submitReviewed(ticket, async () => dto);
  if (answer === "review-stale") {
    pushToast(msg("document-wiring-changed-meanwhile-choose-again", { value1: name }), "info");
    return false;
  }
  if (!answer.took) pushToast(msg("document-wiring-couldn-t-overwrite", { value1: name }), "error");
  return answer.took;
}

/** The live-conflict draft of `name`: its text, the revision it was installed
 * from (the Concord ledger base) and its exact loaded instance. None when the
 * file was deleted on disk: there is no file to review against or open, so the
 * conflict bar's whole-page choices resolve it (GH #541). */
export function liveConflictDraft(name: string): { page: PageDto; baseRev: string | null; generation: number } | null {
  if (!isConflicted(name) || host()?.disk(name)?.kind === "no-file") return null;
  const page = pageToDto(name), generation = pageInstanceGeneration(name);
  return page && generation !== null ? { page, baseRev: baseRev.get(name) ?? null, generation } : null;
}

/** True when two drafts carry the same editable content and identity. */
export function sameLiveDraft(a: PageDto, b: PageDto): boolean {
  const key = (p: PageDto) => JSON.stringify([p.name, p.kind, p.title, p.pre_block, p.blocks, p.format ?? "md"]);
  return key(a) === key(b);
}

/** Apply a reviewed Concord merge (S7): the read-only native merge produces the
 * text, and the client submits it on the reviewed disk state only if no input
 * arrived since the review. "kept": newer input stays, the user reviews again.
 * "saving": the host took the resolution, but its publication is not proved yet. */
export async function applyLiveResolution(name: string, generation: number, reviewed: PageDto, path: string,
  conflictRev: string, mergeBaseRev: string | undefined, decisions: Record<string, MergeDecision>,
  preChoice: "mine" | "theirs" | "union"): Promise<"installed" | "saving" | "kept" | "gone"> {
  const c = host();
  const now = pageToDto(name);
  if (!c || pageInstanceGeneration(name) !== generation || !now || !sameLiveDraft(now, reviewed)) return "kept";
  const ticket = c.reviewTicket(name, conflictRev);
  if (!ticket) return "gone";
  const answer = await c.submitReviewed(ticket,
    () => backend().mergeLiveConflict(path, reviewed, conflictRev, mergeBaseRev, decisions, preChoice));
  if (answer === "review-stale") return "kept";
  if (!answer.took) return "gone";
  // Taken: the page now holds the resolution. A false barrier only means its
  // publication is not proved yet (a save backoff, REVIEW-3b-P2b F2).
  return await settle(c, [name]) ? "installed" : "saving";
}

/** The reviewed disk state as a token ("absent" is a proved missing file). */
export { reviewedDiskToken };

// ------------------------------------------------------------------ creation

export type CreatePageRefusalReason =
  | "name-mismatch" | "page-conflicted" | "page-dirty" | "page-saving" | "stale-binding" | "alias" | "graph-changed" | "graph-rewrite";

/** Local precondition refusal. The `conflict` error family is reserved for a
 * file that exists (or changed) on disk. */
export class CreatePageRefusal extends Error {
  constructor(readonly reason: CreatePageRefusalReason) {
    super(`create-page:${reason}`);
    this.name = "CreatePageRefusal";
  }
}

/** Write a newly authored page through the host and wait until it is published;
 * resolves the published file's revision when the host named it. `baseRev`
 * names the file revision the text was authored on; without it an existing
 * file is a `conflict` and nothing is written. `bindingGeneration` refuses a
 * text authored for another graph binding. */
export async function createPage(name: string, dto: PageDto,
  options: { baseRev?: string | null; bindingGeneration?: number } = {}): Promise<string | null> {
  if (graphRewriteFrozen()) throw new CreatePageRefusal("graph-rewrite");
  if (dto.name !== name) throw new CreatePageRefusal("name-mismatch");
  if (isConflicted(name)) throw new CreatePageRefusal("page-conflicted");
  if (isDirty(name)) throw new CreatePageRefusal("page-dirty");
  if (isSaving(name)) throw new CreatePageRefusal("page-saving");
  if (options.bindingGeneration !== undefined && options.bindingGeneration !== captureBinding().backendGeneration)
    throw new CreatePageRefusal("stale-binding");
  const c = host();
  if (!c) throw new CreatePageRefusal("graph-changed");
  const generation = pageInstanceGeneration(name);
  deletedPages.delete(name);
  const expected: DiskToken | undefined = options.baseRev ? { kind: "file", rev: options.baseRev } : undefined;
  const finish = beginResultsChange();
  let created: Awaited<ReturnType<HostClient["createPage"]>>;
  try { created = await c.createPage(name, dto, { kinds: [expected ? "replace-page" : "create-page"], expected,
    refusedNow: () => graphRewriteFrozen() ? "graph-rewrite" : null }); }
  finally { finish(); }
  if (host() !== c) throw new CreatePageRefusal("graph-changed");
  if (created.kind === "exists") throw new Error("conflict");
  if (created.kind === "unpublished") throw new Error(`Couldn't save “${name}” to disk.`);
  if (created.kind === "refused") {
    const refusal = created.refusal;
    if (refusal === "graph-rewrite") throw new CreatePageRefusal("graph-rewrite");
    if (typeof refusal === "object" && refusal?.reason === "alias") throw new CreatePageRefusal("alias");
    const message = refusal ? refusalMessage(name, refusal) : null;
    throw new Error(refusal ? (message ? renderEnglish(message) : (typeof refusal === "string" ? refusal : refusal.reason))
      : `Couldn't save “${name}”.`);
  }
  if (pageInstanceGeneration(name) === generation && pageByName(name) && !pageByName(name)!.id) setPageId(name, created.key);
  bumpPageInventoryRev();
  bumpDataRev();
  return baseRev.get(name) ?? null;
}

/** Whether `error` is `createPage`'s "the file exists or changed on disk". */
export function isCreateConflict(error: unknown): boolean {
  return errorFamily(error) === "conflict";
}

// ------------------------------------------------------------------ deletion (§12, D4)

/** Delete the page's file through the host: its own input is sent and
 * published first (the host refuses a delete over unsaved input). The host
 * waits for the deletion to publish (Finding B), so only `applied` means the
 * file is gone; `pending`, `uncertain` and `superseded` keep the page and say
 * why. The backend's identity checks reject a twin or a stale `path`; the
 * rejection reaches the caller, which logs it and can report it (I-9). */
export async function deletePageOnDisk(name: string, kind: PageKind, path?: string, reportFailure?: (reason: UiText) => void): Promise<boolean> {
  const c = host();
  if (!c || !await settle(c, [name], { noConflict: true })) {
    const reason = c ? msg("delete-reason-unsaved", { name }) : msg("delete-reason-no-host");
    if (reportFailure) reportFailure(reason);
    else if (c && host() === c) pushToast(reason, "error");
    return false;
  }
  const owner = bindingOwner();
  deletedPages.add(name);
  const finish = beginResultsChange();
  let operation: PageOperation;
  try {
    // A stale delete command deleted nothing the window can count on.
    const remove = () => writeOwned(owner, backend().pageDelete(c.session, name, kind, path))
      .then((done) => done.kind === "current" ? done.value : "refused" as const);
    operation = await remove();
    // The host is still answering this page: once it is clean, once more.
    if (operation === "waiting" && await settle(c, [name])) operation = await remove();
  } catch (error) {
    if (host() === c) deletedPages.delete(name);
    throw error;
  } finally { finish(); }
  if (operation === "applied") return true;
  if (host() === c) deletedPages.delete(name);
  const why = {
    waiting: msg("delete-reason-waiting", { name }),
    pending: msg("delete-reason-pending", { name }),
    uncertain: msg("delete-reason-uncertain", { name }),
    superseded: msg("delete-reason-superseded", { name }),
    refused: msg("delete-reason-refused"),
  }[operation];
  if (reportFailure) reportFailure(why);
  else if (operation !== "refused") pushToast(why, "info");
  return false;
}

// ------------------------------------------------------------------ transfers (§8)

type TransferEdge = readonly [source: string, destination: string];

/** The texts of `names` right before a multi-page edit (next to its undo snapshot). */
export function capturePages(names: Iterable<string>): Map<string, PageDto> {
  const before = new Map<string, PageDto>();
  for (const name of new Set(names)) {
    const dto = pageToDto(name);
    if (dto) before.set(name, dto);
  }
  return before;
}

/** Persist a multi-page edit already applied to the document. Pages with no
 * transfer are ordinary edits; transfer endpoints run as a planned sequence of
 * host moves (freeze, drain, plan, run), the display showing the edit while the
 * host's text is held apart. A refusal stops it: completed steps stand and each
 * endpoint shows the host's latest text (its undo is dropped). */
export function persistTransfer(before: ReadonlyMap<string, PageDto>, kinds: EditKind | EditKinds,
  edges: readonly TransferEdge[] = []): Promise<boolean> {
  const seen = new Set<string>();
  const moves = edges.filter(([source, destination]) => {
    const key = JSON.stringify([source, destination]);
    if (source === destination || seen.has(key) || !before.has(source) || !before.has(destination)
      || !pageWritable(source) || !pageWritable(destination)) return false;
    seen.add(key);
    return true;
  });
  const endpoints = [...new Set(moves.flat())];
  const after = new Map(endpoints.map((name) => [name, pageToDto(name)] as const));
  const c = host();
  const plain = !c || [...after.values()].some((dto) => !dto);
  for (const name of before.keys()) if (plain || !after.has(name)) markDirty(name, kinds);
  if (plain || !endpoints.length) return Promise.resolve(true);
  const list: EditKinds = typeof kinds === "string" ? [kinds] : kinds;
  return transfer(c!, endpoints, before, after as Map<string, PageDto>, moves, list);
}

async function transfer(c: HostClient, endpoints: string[], before: ReadonlyMap<string, PageDto>,
  after: ReadonlyMap<string, PageDto>, moves: readonly TransferEdge[], kinds: EditKinds): Promise<boolean> {
  for (const name of endpoints) {
    transferPages.add(name);
    speculation.set(name, before.get(name)!);
  }
  setHostRevision((n) => n + 1);
  let planned = true;
  let outcome;
  try {
    outcome = await runSequence(c, endpoints, (drained) => {
      const steps = endpoints.every((name) => sameText(drained.get(name)!, before.get(name)!))
        ? planSteps(endpoints, before, after, moves) : null;
      planned = !!steps;
      return steps ?? [];
    }, kinds);
  } finally {
    const held = endpoints.map((name) => [name, speculation.get(name)!] as const);
    for (const name of endpoints) {
      transferPages.delete(name);
      speculation.delete(name);
    }
    // The host's text is the page's text: on success it is the edit itself
    // (compared by block identity, so a successful transfer keeps its nodes).
    for (const [name, text] of held) {
      const shown = client === c && pageByName(name) ? pageToDto(name) : undefined;
      if (shown !== undefined && !(shown && sameText(shown, text))) installPageContent(name, text);
    }
    setHostRevision((n) => n + 1);
    // The endpoints take input again: the storage A/B's `unfreeze` (bench only).
    benchEvent("unfreeze");
  }
  if (outcome.ok && planned) return true;
  const pages = outcome.ok ? endpoints : outcome.pages;
  pushToast(msg("document-move-incomplete", { completed: outcome.completed, pages: pages.join("”, “") }), "error");
  return false;
}

function sameText(a: PageDto, b: PageDto): boolean {
  const shape = (blocks: BlockDto[]): unknown => blocks.map((block) => [block.id, block.raw, block.collapsed, shape(block.children)]);
  return a.pre_block === b.pre_block && JSON.stringify(shape(a.blocks)) === JSON.stringify(shape(b.blocks));
}

function idsOf(blocks: readonly BlockDto[], into = new Set<string>()): Set<string> {
  for (const block of blocks) {
    into.add(block.id);
    idsOf(block.children, into);
  }
  return into;
}

/** The maximal subtrees of `blocks` whose root came from `from`. */
function unitsFrom(blocks: readonly BlockDto[], from: Set<string>, into: BlockDto[] = []): BlockDto[] {
  for (const block of blocks) {
    if (from.has(block.id)) into.push(block);
    else unitsFrom(block.children, from, into);
  }
  return into;
}

function without(blocks: readonly BlockDto[], ids: Set<string>): BlockDto[] {
  return blocks.filter((block) => !ids.has(block.id))
    .map((block) => ({ ...block, children: without(block.children, ids) }));
}

/** Step i moves its edge's blocks: every page's text is the edit's result minus
 * the blocks later steps bring in, plus the blocks later steps take out
 * (appended at the end: a transitional state a crash between steps can leave).
 * Null when the edges do not partition the moved blocks (nothing is sent). */
function planSteps(endpoints: string[], before: ReadonlyMap<string, PageDto>, after: ReadonlyMap<string, PageDto>,
  moves: readonly TransferEdge[]): SequenceStep[] | null {
  const units = moves.map(([source, destination]) => unitsFrom(after.get(destination)!.blocks, idsOf(before.get(source)!.blocks)));
  const textAt = (name: string, done: number): PageDto => {
    const arriving = new Set<string>();
    const leaving: BlockDto[] = [];
    moves.forEach(([source, destination], j) => {
      if (j < done) return;
      if (destination === name) for (const unit of units[j]) arriving.add(unit.id);
      if (source === name) leaving.push(...units[j]);
    });
    const dto = after.get(name)!;
    return { ...dto, blocks: [...without(dto.blocks, arriving), ...leaving] };
  };
  const census = (done: number) => endpoints.flatMap((name) => [...idsOf(textAt(name, done).blocks)]).sort().join("\u0000");
  const whole = census(moves.length);
  for (let done = 0; done < moves.length; done += 1) if (census(done) !== whole) return null;
  return moves.map(([source, destination], i) => ({
    source, receiver: destination, sourceDto: textAt(source, i + 1), receiverDto: textAt(destination, i + 1),
  }));
}

// ------------------------------------------------------------------ alias drafts (§8)

let aliasDraftRouteHandler: ((name: string, kind: PageKind) => void) | null = null;
export function installAliasDraftRouteHandler(handler: (name: string, kind: PageKind) => void): void {
  aliasDraftRouteHandler = handler;
}

function queueAliasLanding(name: string, owners: string[]): void {
  const c = host();
  if (!c || !owners[0] || landing.has(name) || !pageByName(name) || !c.isDirty(name)) return;
  landing.add(name);
  void landAliasDraft(c, name, owners[0]).finally(() => landing.delete(name));
}

/** A page with no file whose name became an alias: its text belongs to the
 * owner. The owner's live text gets the draft (or has its landed copy
 * replaced), then the draft leaves for the owner's route. */
async function landAliasDraft(c: HostClient, name: string, ownerPath: string): Promise<void> {
  const generation = pageInstanceGeneration(name);
  let owned;
  try { owned = await readOwned(bindingOwner(), backend().getPageByPath(ownerPath)); }
  catch {
    // The draft stays in the editor, unsaved and listed; say why it did not move.
    pushToast(msg("document-wiring-couldn-t-read-the-page-is-an-alias-of-its", { value1: ownerPath, value2: name }), "error");
    return;
  }
  const read = owned.kind === "current" ? owned.value : null;
  if (!read || read.read_only || read.guide || host() !== c) return;
  const refused = ensurePageLoaded(read);
  if (refused) { reportPageLoadRefusal(refused); return; }
  const owner = read.name;
  const release = c.acquire(owner, { pin: true });
  try {
    await c.drain([owner]);
    const draft = pageToDto(name);
    const ownerText = c.doc.dto(owner);
    if (!draft || !ownerText || host() !== c || pageInstanceGeneration(name) !== generation
      || !c.isOpen(owner) || c.busy(owner) || c.conflicted(owner)) return;
    // An editor open on the owner may hold input not yet in the store (an IME
    // composition is DOM-local): replacing the owner's text under it would
    // drop that input. The draft stays; its next edit lands again (P9).
    const editing = editingId();
    if (editing && doc.byId[editing]?.page === owner) return;
    const landed = landedAliasDrafts.get(name);
    const text = landed && landed.generation === generation
      ? landed.owner === ownerPath ? replaceLandedAliasDraft(ownerText, landed.blocks, draft) : null
      : appendAliasDraft(ownerText, draft);
    if (!text) {
      pushToast(msg("document-wiring-changed-since-was-added-to-it-copy-the-rest-of", { value1: owner, value2: name, value3: name }), "error");
      return;
    }
    installPageContent(owner, text);
    c.noteEdit(owner, ["insert-blocks"]);
    if (!await settle(c, [owner])) return;
    landedAliasDrafts.set(name, { owner: ownerPath, blocks: aliasDraftBlocks(draft), generation });
    const now = pageToDto(name);
    if (!now || !sameLiveDraft(now, draft) || host() !== c) return;
    landedAliasDrafts.delete(name);
    c.forget(name);
    forgetPage(name);
    aliasDraftRouteHandler?.(owner, read.kind);
    pushToast(msg("document-wiring-moved-into-its-alias-owner", { value1: name, value2: owner }), "info");
    bumpPageInventoryRev();
  } finally {
    release();
  }
}

// The command line's writes through this graph's owner (CLI slice 2,
// `src-tauri/src/owner/window.rs`). Before a write reads a page, the owner
// asks the window to save the text typed on it: the host only sees input the
// window has submitted, and typing is debounced. The window runs its page
// barrier and answers whether that text is saved.

/** Answer the owner's flush requests for the graph this window is bound to; a
 *  stale binding's request gets no answer (the owner then refuses the write). */
export function subscribeOwnerFlush(): () => void {
  let alive = true;
  let unsub = () => {};
  const subscribe = backend().onOwnerFlushRequest;
  if (!subscribe) return () => {};
  void readOwnedResource(
    ownedWhen(() => alive),
    subscribe.call(backend(), (request) => {
      if (request.binding_generation !== captureBinding().backendGeneration) return;
      void answerOwnerFlush(request.id, request.page, request.binding_generation);
    }),
    (stop) => stop(),
  ).then((result) => { if (result.kind === "current") unsub = result.value; });
  return () => { alive = false; unsub(); };
}

async function answerOwnerFlush(id: number, page: string, generation: number): Promise<void> {
  let saved: boolean;
  try {
    saved = await flushPage(page);
  } catch {
    saved = false; // a failed barrier is unsaved text, never success
  }
  // Text a component holds outside the document (an IME composition, a
  // title or sheet-cell edit) is not saved by the barrier (review 5 R2).
  if (draftPinned(page)) saved = false;
  // A rebinding during the barrier leaves the request with no answer: the
  // owner it came from refuses its write (review 5 R3).
  if (generation !== captureBinding().backendGeneration) return;
  const answer = backend().ownerFlushAnswer?.(id, saved, generation);
  if (answer) void readOwned(bindingOwner(), answer).catch(ignoreLostFlushAnswer);
}
