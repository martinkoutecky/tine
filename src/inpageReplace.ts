import { msg } from "./i18n";
// In-page find & replace (GH #208, #338). Replace edits only the outline blocks of
// the pane find was opened in: never sidebar items, query/reference rows or the
// rest of the graph. Every write is the ordinary block edit (`setRaw`, the call
// the editor's commit makes) inside one `withUndoUnit`, so writability, undo,
// the page host's base-revision-guarded save and external-change handling
// behave exactly as typing does. This module never touches a file.
//
// Find matches what a block SHOWS (rendered text); an edit changes what the
// editor shows (raw minus hidden property lines). A rendered occurrence is
// replaced only after a re-render proves the edit lands on that occurrence: a
// private-use sentinel goes where the replacement would, and the block must then
// render as the original with exactly that match swapped for the sentinel.
// Hidden properties (`id::`, `collapsed::`) are never in the edited text, and an
// occurrence find did not show (a link target) can never pass the proof.
import { createMemo, createRoot, createSignal } from "solid-js";
import { blockWritable, formatForBlock, node as docNode, setRaw, withUndoUnit } from "./document";
import { annotationInfo } from "./editor/annotation";
import { BUILTIN_HIDE, HIDE_ALL, joinProps, splitProps, visibleOffsetsToRawBytes, type HidePolicy } from "./editor/properties";
import { facetsOf } from "./render/facets";
import { drawnSourceByBlock, utf8ByteLength, type DrawnSource } from "./render/spans";
import type { Format } from "./types";
import {
  activateInPageFindIndex,
  findScopeElement,
  findTextOccurrences,
  inPageFindActiveIndex,
  inPageFindBlockText,
  inPageFindMatches,
  inPageFindPaneScopeId,
  inPageFindQuery,
  inPageFindRenderedText,
  type InPageFindMatch,
} from "./inpageFind";
import { searchRemoveAccents } from "./ui";
import { pushToast } from "./toasts";

type Span = { start: number; end: number };

export interface BlockReplacementPlan {
  /** The block's new raw (the old raw when nothing was replaced). */
  raw: string;
  replaced: number;
  /** Matches find showed that could not be proven to be an editable occurrence. */
  skipped: number;
}

const SENTINEL_BASE = 0xe000;
const SENTINEL_LIMIT = 0x1900; // the BMP private-use area: U+E000..U+F8FF
/** Candidate occurrences tried per match when the order-preserving alignment
 *  does not prove out; bounds a pathological block to O(matches × this) parses. */
const MAX_CANDIDATES_PER_MATCH = 8;

/** Order-preserving map from rendered offsets onto the visible text: each
 *  rendered character takes the next equal visible character, or -1 when it has
 *  none (a glyph the source spells differently, like `\alpha`). O(visible) per
 *  mapped character, linear when rendered text is visible text minus markup. */
function alignRenderedToVisible(rendered: string, visible: string): Int32Array {
  const map = new Int32Array(rendered.length).fill(-1);
  let from = 0;
  for (let i = 0; i < rendered.length; i++) {
    const at = visible.indexOf(rendered[i], from);
    if (at < 0) continue;
    map[i] = at;
    from = at + 1;
  }
  return map;
}

function alignedSpan(map: Int32Array, match: Span): Span | null {
  const start = map[match.start];
  if (start < 0) return null;
  for (let i = match.start + 1; i < match.end; i++) if (map[i] !== start + (i - match.start)) return null;
  return { start, end: start + (match.end - match.start) };
}

function splice(text: string, spans: readonly Span[], insert: (index: number) => string): string {
  let out = text;
  for (let i = spans.length - 1; i >= 0; i--) out = out.slice(0, spans[i].start) + insert(i) + out.slice(spans[i].end);
  return out;
}

const overlaps = (a: Span, b: Span) => a.start < b.end && b.start < a.end;

type Accepted = { target: number; span: Span };

/** Whether `s` overlaps a span of `sorted` (disjoint, by start). O(log n). */
function overlapsAny(sorted: readonly Accepted[], s: Span): boolean {
  let lo = 0, hi = sorted.length;
  while (lo < hi) { const mid = (lo + hi) >> 1; if (sorted[mid].span.end <= s.start) lo = mid + 1; else hi = mid; }
  return lo < sorted.length && sorted[lo].span.start < s.end;
}

function insertSorted(sorted: readonly Accepted[], item: Accepted): Accepted[] {
  let at = sorted.findIndex((a) => a.span.start > item.span.start);
  if (at < 0) at = sorted.length;
  return [...sorted.slice(0, at), item, ...sorted.slice(at)];
}

/** The property lines Replace's editor text leaves out (an annotation hides all). */
const editHidePolicy = (raw: string, format: Format): HidePolicy =>
  annotationInfo(facetsOf(raw, format).properties) ? HIDE_ALL : BUILTIN_HIDE;

/** Plan replacing find's rendered occurrences `ordinals` (or all) of `query` in
 *  one block. Pure: reads nothing but its arguments and the parser. A span is
 *  written only when `drawn` says the pane draws its source bytes as themselves
 *  ({@link drawnSourceByBlock}); every requested match not written is counted
 *  as skipped, including one the block's text no longer holds. Cost: one split,
 *  one batched offset mapping and one proof render; only a match whose aligned
 *  span is missing, not drawn or not proven tries up to MAX_CANDIDATES_PER_MATCH
 *  other drawn occurrences, one proof each, then one joint proof. */
export function planBlockReplacement(
  raw: string,
  format: Format,
  query: string,
  replacement: string,
  ordinals: readonly number[] | "all",
  removeAccents: boolean,
  drawn: DrawnSource,
  renderedText: string = inPageFindRenderedText(raw, format),
): BlockReplacementPlan {
  const q = query.trim();
  const shown = q ? findTextOccurrences(renderedText, q, removeAccents) : [];
  const asked = ordinals === "all" ? shown.length : new Set(ordinals).size;
  const unchanged = (): BlockReplacementPlan => ({ raw, replaced: 0, skipped: asked });
  const targets = (ordinals === "all" ? shown.map((_, i) => i) : [...new Set(ordinals)].filter((i) => i >= 0 && i < shown.length))
    .sort((a, b) => a - b);
  if (!targets.length || targets.length > SENTINEL_LIMIT) return unchanged();
  const hide = editHidePolicy(raw, format);
  const { visible, hidden } = splitProps(raw, hide, format);
  // Refusal (in-scope scenario: malformed or hand-edited imported Markdown/Org
  // whose property lines are not where the editor puts them): rejoining would
  // rewrite bytes the user never touched, so the block is left as it is.
  if (joinProps(visible, hidden, format, raw, hide) !== raw) return unchanged();
  const sentinel = (i: number) => String.fromCharCode(SENTINEL_BASE + i);
  for (let i = 0; i < targets.length; i++) if (raw.includes(sentinel(i))) return unchanged();

  const proves = (spans: readonly Span[], matchIndexes: readonly number[]): boolean => {
    const candidate = joinProps(splice(visible, spans, sentinel), hidden, format, raw, hide);
    const expected = splice(renderedText, matchIndexes.map((m) => shown[m]), sentinel);
    return inPageFindRenderedText(candidate, format) === expected;
  };

  const map = alignRenderedToVisible(renderedText, visible);
  const aligned = targets.map((t) => alignedSpan(map, shown[t]));
  const occurrences = findTextOccurrences(visible, q, removeAccents);
  // Every span end either pass may write, mapped to source bytes in one call.
  const ends = [...new Set([...aligned, ...occurrences].flatMap((s) => (s ? [s.start, s.end] : [])))].sort((a, b) => a - b);
  const rawBytes = visibleOffsetsToRawBytes(raw, ends, hide, format);
  const lead = utf8ByteLength(raw) - utf8ByteLength(raw.trimStart());
  // Inverse of rebulletedSourceByteToRawByte (the parser sees a "- " bullet).
  const sourceOf = new Map(ends.map((offset, i) => [offset, rawBytes[i] - lead + 2]));
  const writable = (span: Span) => drawn([sourceOf.get(span.start)!, sourceOf.get(span.end)!]);

  const first = targets.flatMap((target, i) => {
    const span = aligned[i];
    return span && writable(span) ? [{ target, span }] : [];
  });
  const proven = first.length && proves(first.map((a) => a.span), first.map((a) => a.target)) ? first : [];
  // Only matches the joint proof did not settle look further: the drawn
  // occurrences no accepted span overlaps, first come first (as develop).
  let accepted = [...proven];
  const settled = new Set(proven.map((a) => a.target));
  const pool = occurrences.filter((s) => writable(s) && !overlapsAny(accepted, s));
  let added = false;
  for (let i = 0; i < targets.length; i++) {
    const target = targets[i];
    if (settled.has(target)) continue;
    const own = aligned[i] && writable(aligned[i]!) && !overlapsAny(accepted, aligned[i]!) ? [aligned[i]!] : [];
    const candidates = own.concat(pool.slice(0, MAX_CANDIDATES_PER_MATCH + 1).filter((s) => s.start !== own[0]?.start))
      .slice(0, MAX_CANDIDATES_PER_MATCH);
    const span = candidates.find((c) => proves([c], [target]));
    if (!span) continue;
    accepted = insertSorted(accepted, { target, span });
    for (let k = pool.length - 1; k >= 0; k--) if (overlaps(pool[k], span)) pool.splice(k, 1);
    added = true;
  }
  // The individually proven edits must also hold together.
  if (added && !proves(accepted.map((a) => a.span), accepted.map((a) => a.target))) accepted = proven;
  if (!accepted.length) return unchanged();
  const next = joinProps(splice(visible, accepted.map((a) => a.span), () => replacement), hidden, format, raw, hide);
  return { raw: next, replaced: accepted.length, skipped: asked - accepted.length };
}

export interface InPageReplaceOutcome {
  replaced: number;
  skipped: number;
  /** Why nothing was written, when that is the answer. */
  refused?: "no-match" | "other-scope" | "read-only" | "unprovable";
}

const replaceState = createRoot(() => {
  const [open, setOpen] = createSignal(false);
  const [text, setText] = createSignal("");
  const [skippedAt, setSkippedAt] = createSignal<{ count: number; query: string; matches: string } | null>(null);
  // Shown while the query and Find's outline matches are the ones the last
  // Replace left; any other change (a new query, an edit) clears it.
  const skippedHere = createMemo(() => {
    const at = skippedAt();
    return at && at.query === inPageFindQuery() && at.matches === matchListKey(inPageFindMatches()) ? at.count : 0;
  });
  return { open, setOpen, text, setText, setSkippedAt, skippedHere };
});
/** Whether the find panel shows its replace row (session-only UI state). */
export const inPageReplaceOpen = replaceState.open;
export const setInPageReplaceOpen = replaceState.setOpen;
export const inPageReplaceText = replaceState.text;
export const setInPageReplaceText = replaceState.setText;
/** How many matches the last Replace or Replace all left unwritten, while the
 *  query and the match list are still the ones it left (0 otherwise). */
export const inPageReplaceSkippedHere = replaceState.skippedHere;

function matchListKey(matches: readonly InPageFindMatch[]): string {
  return matches.filter((m) => !m.surfaceId).map((m) => `${m.scopeId}\0${m.blockId}\0${m.ordinalInBlock}\0${m.start}`).join("\n");
}

function finish(outcome: InPageReplaceOutcome, query: string): InPageReplaceOutcome {
  replaceState.setSkippedAt(outcome.skipped ? { count: outcome.skipped, query, matches: matchListKey(inPageFindMatches()) } : null);
  return outcome;
}

function replaceableInScope(match: InPageFindMatch, scopeId: string): boolean {
  return !match.surfaceId && (match.scopeId ?? scopeId) === scopeId && !!docNode(match.blockId);
}

/** Plan every block, then write them as ONE undo step through setRaw. Every
 *  requested match is either written or counted in `skipped`. Returns null when
 *  the undo unit refused (a frozen graph rewrite or a read-only page). */
function applyPlans(byBlock: Map<string, readonly number[]>, query: string, replacement: string, scopeId: string): InPageReplaceOutcome | null {
  const removeAccents = searchRemoveAccents();
  let skipped = 0;
  const writes: { id: string; raw: string; replaced: number }[] = [];
  const raws = new Map<string, string>();
  for (const id of byBlock.keys()) { const node = docNode(id); if (node) raws.set(id, node.raw); }
  // A block the pane does not draw as itself (lazy, under a fold, open in the
  // editor, a snapshot, a shell) has no entry: its matches are skipped.
  const pane = findScopeElement(scopeId);
  const drawnBy = pane ? drawnSourceByBlock(pane, raws) : new Map<string, DrawnSource>();
  const nothing: DrawnSource = () => false;
  for (const [id, ordinals] of byBlock) {
    const raw = raws.get(id);
    if (raw === undefined) { skipped += ordinals.length; continue; }
    const format = formatForBlock(id);
    let plan: BlockReplacementPlan;
    try {
      plan = planBlockReplacement(raw, format, query, replacement, ordinals, removeAccents, drawnBy.get(id) ?? nothing,
        inPageFindBlockText(id, raw, format));
    } catch {
      // The kernel refused to rejoin this block's hidden lines (its parse is
      // quarantined): nothing of it is written, and its matches are counted.
      skipped += ordinals.length;
      continue;
    }
    skipped += plan.skipped;
    if (plan.replaced && plan.raw !== raw) writes.push({ id, raw: plan.raw, replaced: plan.replaced });
  }
  if (!writes.length) return { replaced: 0, skipped, ...(skipped ? { refused: "unprovable" as const } : { refused: "no-match" as const }) };
  const pages = [...new Set(writes.map((w) => docNode(w.id)!.page))];
  const done = withUndoUnit("find-replace", pages, () => {
    for (const w of writes) setRaw(w.id, w.raw);
    return true;
  });
  if (done !== true) return null;
  return { replaced: writes.reduce((n, w) => n + w.replaced, 0), skipped };
}

function reportRefusedUnit(skipped: number): InPageReplaceOutcome {
  pushToast(msg("inpage-replace-nothing-was-replaced-this-page-cannot-be-edited"), "error");
  return { replaced: 0, skipped, refused: "read-only" };
}

/** Replace find's active match, then move to the next match. */
export function replaceActiveInPageFindMatch(replacement = inPageReplaceText()): InPageReplaceOutcome {
  const matches = inPageFindMatches();
  const index = inPageFindActiveIndex();
  const match = matches[index];
  const query = inPageFindQuery();
  if (!match) return { replaced: 0, skipped: 0, refused: "no-match" };
  const scopeId = inPageFindPaneScopeId();
  if (!replaceableInScope(match, scopeId)) {
    pushToast(msg("inpage-replace-replace-changes-only-the-page-you-opened-find"), "info");
    activateInPageFindIndex(index + 1);
    return finish({ replaced: 0, skipped: 1, refused: "other-scope" }, query);
  }
  if (!blockWritable(match.blockId)) return finish(reportRefusedUnit(1), query);
  const outcome = applyPlans(new Map([[match.blockId, [match.ordinalInBlock]]]), query, replacement, scopeId);
  if (!outcome) return finish(reportRefusedUnit(1), query);
  if (!outcome.replaced) {
    pushToast(msg("inpage-replace-this-match-is-inside-a-link-or-markup"), "info");
    activateInPageFindIndex(index + 1);
    return finish(outcome, query);
  }
  // The replacement's own occurrences stay behind the caret: skip past them.
  const reintroduced = findTextOccurrences(replacement, query.trim(), searchRemoveAccents()).length;
  activateInPageFindIndex(index + reintroduced);
  return finish(outcome, query);
}

/** Replace every match find shows on the page it was opened in, as one undo
 *  step. Every match Find holds is written or counted as skipped. */
export function replaceAllInPageFindMatches(replacement = inPageReplaceText()): InPageReplaceOutcome {
  const query = inPageFindQuery();
  const scopeId = inPageFindPaneScopeId();
  const matches = inPageFindMatches();
  const byBlock = new Map<string, number[]>();
  let readOnly = 0;
  let elsewhere = 0;
  for (const match of matches) {
    if (!replaceableInScope(match, scopeId)) { elsewhere++; continue; }
    if (!blockWritable(match.blockId)) { readOnly++; continue; }
    const ordinals = byBlock.get(match.blockId);
    if (ordinals) ordinals.push(match.ordinalInBlock);
    else byBlock.set(match.blockId, [match.ordinalInBlock]);
  }
  if (!byBlock.size) {
    if (readOnly) return finish(reportRefusedUnit(matches.length), query);
    pushToast(msg("inpage-replace-no-matches-on-this-page-to-replace"), "info");
    return finish({ replaced: 0, skipped: elsewhere, refused: "no-match" }, query);
  }
  const outcome = applyPlans(byBlock, query, replacement, scopeId);
  if (!outcome) return finish(reportRefusedUnit(matches.length), query);
  const skipped = outcome.skipped + readOnly + elsewhere;
  if (!outcome.replaced) {
    pushToast(msg("inpage-replace-skipped", { count: skipped }), "info");
  } else {
    pushToast(msg("inpage-replace-result", { count: outcome.replaced, skipped }), "success");
  }
  activateInPageFindIndex(0);
  return finish({ ...outcome, skipped }, query);
}
