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
import { createRoot, createSignal } from "solid-js";
import { blockWritable, formatForBlock, node as docNode, setRaw, withUndoUnit } from "./document";
import { annotationInfo } from "./editor/annotation";
import { BUILTIN_HIDE, HIDE_ALL, joinProps, splitProps } from "./editor/properties";
import { facetsOf } from "./render/facets";
import type { Format } from "./types";
import {
  activateInPageFindIndex,
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

/** Plan replacing find's rendered occurrences `ordinals` (or all) of `query` in
 *  one block. Pure: reads nothing but its arguments and the parser. Cost: one
 *  split and one proof render when the alignment holds; otherwise O(matches ×
 *  MAX_CANDIDATES_PER_MATCH) proof renders. */
export function planBlockReplacement(
  raw: string,
  format: Format,
  query: string,
  replacement: string,
  ordinals: readonly number[] | "all",
  removeAccents: boolean,
  renderedText: string = inPageFindRenderedText(raw, format),
): BlockReplacementPlan {
  const q = query.trim();
  const unchanged = (skipped: number): BlockReplacementPlan => ({ raw, replaced: 0, skipped });
  const shown = q ? findTextOccurrences(renderedText, q, removeAccents) : [];
  const targets = (ordinals === "all" ? shown.map((_, i) => i) : [...new Set(ordinals)].filter((i) => i >= 0 && i < shown.length))
    .sort((a, b) => a - b);
  if (!targets.length) return unchanged(0);
  if (targets.length > SENTINEL_LIMIT) return unchanged(targets.length);
  const hide = annotationInfo(facetsOf(raw, format).properties) ? HIDE_ALL : BUILTIN_HIDE;
  const { visible, hidden } = splitProps(raw, hide, format);
  // Refusal (in-scope scenario: malformed or hand-edited imported Markdown/Org
  // whose property lines are not where the editor puts them): rejoining would
  // rewrite bytes the user never touched, so the block is left as it is.
  if (joinProps(visible, hidden, format) !== raw) return unchanged(targets.length);
  const sentinel = (i: number) => String.fromCharCode(SENTINEL_BASE + i);
  for (let i = 0; i < targets.length; i++) if (raw.includes(sentinel(i))) return unchanged(targets.length);

  const proves = (spans: readonly Span[], matchIndexes: readonly number[]): boolean => {
    const candidate = joinProps(splice(visible, spans, sentinel), hidden, format);
    const expected = splice(renderedText, matchIndexes.map((m) => shown[m]), sentinel);
    return inPageFindRenderedText(candidate, format) === expected;
  };

  const map = alignRenderedToVisible(renderedText, visible);
  const aligned = targets.map((t) => alignedSpan(map, shown[t]));
  let accepted: { target: number; span: Span }[] = [];
  if (aligned.every((s): s is Span => !!s) && proves(aligned, targets)) {
    accepted = targets.map((target, i) => ({ target, span: aligned[i]! }));
  } else {
    const occurrences = findTextOccurrences(visible, q, removeAccents);
    for (let i = 0; i < targets.length; i++) {
      const target = targets[i];
      const free = (s: Span) => !accepted.some((a) => overlaps(a.span, s));
      const candidates = [aligned[i], ...occurrences].filter((s): s is Span => !!s && free(s)).slice(0, MAX_CANDIDATES_PER_MATCH);
      const proven = candidates.find((span) => proves([span], [target]));
      if (proven) accepted.push({ target, span: proven });
    }
    accepted.sort((a, b) => a.span.start - b.span.start);
    // The individually proven edits must also hold together.
    if (accepted.length && !proves(accepted.map((a) => a.span), accepted.map((a) => a.target))) accepted = [];
  }
  if (!accepted.length) return unchanged(targets.length);
  const next = joinProps(splice(visible, accepted.map((a) => a.span), () => replacement), hidden, format);
  return { raw: next, replaced: accepted.length, skipped: targets.length - accepted.length };
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
  return { open, setOpen, text, setText };
});
/** Whether the find panel shows its replace row (session-only UI state). */
export const inPageReplaceOpen = replaceState.open;
export const setInPageReplaceOpen = replaceState.setOpen;
export const inPageReplaceText = replaceState.text;
export const setInPageReplaceText = replaceState.setText;

function replaceableInScope(match: InPageFindMatch, scopeId: string): boolean {
  return !match.surfaceId && (match.scopeId ?? scopeId) === scopeId && !!docNode(match.blockId);
}

/** Plan every block, then write them as ONE undo step through setRaw. Returns
 *  null when the undo unit refused (a frozen graph rewrite or a read-only page). */
function applyPlans(byBlock: Map<string, readonly number[] | "all">, query: string, replacement: string): InPageReplaceOutcome | null {
  const removeAccents = searchRemoveAccents();
  let skipped = 0;
  const writes: { id: string; raw: string; replaced: number }[] = [];
  for (const [id, ordinals] of byBlock) {
    const node = docNode(id);
    if (!node) continue;
    const format = formatForBlock(id);
    const plan = planBlockReplacement(node.raw, format, query, replacement, ordinals, removeAccents,
      inPageFindBlockText(id, node.raw, format));
    skipped += plan.skipped;
    if (plan.replaced && plan.raw !== node.raw) writes.push({ id, raw: plan.raw, replaced: plan.replaced });
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

function reportRefusedUnit(): InPageReplaceOutcome {
  pushToast(msg("inpage-replace-nothing-was-replaced-this-page-cannot-be-edited"), "error");
  return { replaced: 0, skipped: 0, refused: "read-only" };
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
    return { replaced: 0, skipped: 1, refused: "other-scope" };
  }
  if (!blockWritable(match.blockId)) return reportRefusedUnit();
  const outcome = applyPlans(new Map([[match.blockId, [match.ordinalInBlock]]]), query, replacement);
  if (!outcome) return reportRefusedUnit();
  if (!outcome.replaced) {
    pushToast(msg("inpage-replace-this-match-is-inside-a-link-or-markup"), "info");
    activateInPageFindIndex(index + 1);
    return outcome;
  }
  // The replacement's own occurrences stay behind the caret: skip past them.
  const reintroduced = findTextOccurrences(replacement, query.trim(), searchRemoveAccents()).length;
  activateInPageFindIndex(index + reintroduced);
  return outcome;
}

/** Replace every match find shows on the page it was opened in, as one undo step. */
export function replaceAllInPageFindMatches(replacement = inPageReplaceText()): InPageReplaceOutcome {
  const query = inPageFindQuery();
  const scopeId = inPageFindPaneScopeId();
  const byBlock = new Map<string, number[]>();
  let readOnly = 0;
  for (const match of inPageFindMatches()) {
    if (!replaceableInScope(match, scopeId)) continue;
    if (!blockWritable(match.blockId)) { readOnly++; continue; }
    const ordinals = byBlock.get(match.blockId);
    if (ordinals) ordinals.push(match.ordinalInBlock);
    else byBlock.set(match.blockId, [match.ordinalInBlock]);
  }
  if (!byBlock.size) {
    if (readOnly) return reportRefusedUnit();
    pushToast(msg("inpage-replace-no-matches-on-this-page-to-replace"), "info");
    return { replaced: 0, skipped: 0, refused: "no-match" };
  }
  const outcome = applyPlans(byBlock, query, replacement);
  if (!outcome) return reportRefusedUnit();
  const skipped = outcome.skipped + readOnly;
  if (!outcome.replaced) {
    pushToast(msg("inpage-replace-skipped", { count: skipped }), "info");
  } else {
    pushToast(msg("inpage-replace-result", { count: outcome.replaced, skipped }), "success");
  }
  activateInPageFindIndex(0);
  return { ...outcome, skipped };
}
