// What a Block needs from the margin dialogue (vision §3.7), kept out of
// Block.tsx so the component only wires it (the size ratchet, PARITY-CAMPAIGN §3).

import { createMemo, getOwner, runWithOwner, useContext, type Accessor } from "solid-js";
import type { OutlineScope } from "../document";
import { storedAuthorOf, quoteSelectorOf } from "../comments";
import { authorTint, isOwnAuthor } from "../commentIdentity";
import { marginNext, marginPrev, outlineNext, outlinePrev, threadRootOf } from "../margin";
import { inCommentThread } from "./CommentParts";
import { MarginContext } from "./marginContext";
import { commentMarginHidden } from "../commentMarginPref";
import { authorRunContinues, authorThreadSides, lastRow, type AuthorNeighbors, type RowSides } from "./authorRuns";
import { threadResolved } from "../commentResolution";
import { useResolvedComments } from "./CommentResolutionScope";
import { editingId } from "../editorController";
import { createCommentChildren, createCommentIndexFor } from "../commentIndex";
import { NO_COMMENT_CHILDREN } from "../commentChildren";
import { createQuoteCoverage } from "../render/quoteHighlight";
import { datedThreadRoot, hasMessageDate } from "../document";
export { commentEditorHide } from "../editor/commentProperties";

interface CommentNode { id: string; raw: string; parent: string | null; children: readonly string[] }
type Props = readonly (readonly [string, string])[];
const NO_IDS: readonly string[] = [];
const NO_COMMENT_CLASSES: Record<string, boolean> = {};
const NO_SIDES: RowSides = {};
const NO_THREAD = { continued: false, continues: false } as const;

/** What `commentFacts` needs from a row to compare it with its neighbours: its own id, the sides its list gives
 * it, the first child it renders (none when folded) and whether a row's children are folded away here. */
export interface RunRow { id: () => string; neighbours: () => AuthorNeighbors | undefined; firstChild: () => string | undefined; folded: (id: string) => boolean }

/** Local facets and a memo for the adjacent authorship pairs. The thread test
 * walks up the parents rather than providing a context per level (128-level
 * outlines must render). Unauthored blocks do no neighbour comparisons. */
export function commentFacts(node: () => CommentNode, properties: () => Props, fmt: () => "md" | "org", run?: RunRow) {
  const inThread = () => inCommentThread(node().parent, fmt());
  const commentSelector = () => (node().parent !== null ? quoteSelectorOf(properties()) : null);
  // Carry stored attribution through the memo that already owns adjacent-run comparisons.
  const authorship = createMemo(() => {
    const stored = storedAuthorOf(properties());
    const written = stored?.trim() || null;
    const author = written !== null && isOwnAuthor(written) ? null : written;
    // Thread presentation is gated below. Do not read ancestors here: their
    // raw text would make a parent's keystroke recompute every descendant run.
    if (author === null || commentSelector() !== null || !run) return { stored, author, continued: false, continues: false };
    const sides = run.neighbours()?.() ?? NO_SIDES, id = run.id();
    return { stored, author, continued: authorRunContinues(sides.prev, id, fmt()), continues: authorRunContinues(id, sides.next, fmt()) };
  });
  const storedAuthor = () => authorship().stored;
  const author = () => authorship().author;
  const continued = () => authorship().continued;
  const continues = () => authorship().continues;
  // The thread's run reaches across depth (the parent above the first child, the last descendant above the next
  // sibling), so it depends on more rows than the card. It is read where the classes are, not in a memo of its
  // own: a memo would cost every plain row an owner (Block.loadCost), and keeping it out of the card memo stops
  // a parent's keystroke from re-running the card comparisons of its children. Plain rows read nothing.
  // The row above is the previous sibling's last drawn row. It is memoized, created on the first authored row's
  // read (plain rows pay nothing) and under the row's own owner, and it reads comment membership from the
  // covering index (`isComment`), not from the siblings' text: typing re-runs none of it.
  const owner = getOwner();
  let previousLast: Accessor<string | undefined> | undefined;
  const previousRow = () => (previousLast ??= runWithOwner(owner, () => {
    // The index that covers the previous sibling covers the whole spine below it.
    const index = createCommentIndexFor(() => run!.neighbours()?.()?.prev ?? null);
    // A fold reads the row's raw text; one memo per visited row (under the row's owner, so a rerun does not
    // dispose it) hands the walk a boolean, so typing in a spine row does not re-walk the spine.
    const folds = new Map<string, Accessor<boolean>>();
    const foldOf = (id: string) => {
      let fold = folds.get(id);
      if (!fold) folds.set(id, fold = runWithOwner(owner, () => createMemo(() => run!.folded(id)))!);
      return fold();
    };
    return createMemo(() => {
      const sides = run!.neighbours()?.() ?? NO_SIDES;
      return sides.prev === undefined || sides.results ? undefined
        : lastRow(sides.prev, foldOf, id => index()?.isComment(id) ?? false);
    });
  }))!();
  const threadSides = () => {
    if (!run || author() === null || commentSelector() !== null || bulletless()) return NO_THREAD;
    return authorThreadSides(run.id(), run.neighbours()?.() ?? NO_SIDES, run.firstChild(), previousRow(), fmt());
  };
  // Every hit of a results list starts with its own chip: the tint card may still join adjacent hits (the
  // stylesheet hides the chip of a joined one under the tint), but the thread never links across hits.
  const inResults = () => !!run?.neighbours()?.()?.results;
  const showsAuthor = () => (author() !== null && (!continued() || inResults())) || bulletless();
  // Comments and their replies draw no bullet; the chip keeps the block menu.
  const bulletless = () => (commentSelector() !== null || inThread()) && (!datedThreadRoot(node().id) || hasMessageDate(node().id));
  const commentClasses = () => {
    const by = author();
    const selector = commentSelector();
    const inReplies = inThread();
    if (by === null && selector === null && !inReplies) return NO_COMMENT_CLASSES;
    const thread = threadSides();
    const classes: Record<string, boolean> = {
      "comment-card": selector !== null && !inReplies,
      "comment-resolved": selector !== null && !inReplies && threadResolved(node().id),
      "comment-in-thread": bulletless(),
      "comment-message": bulletless() && datedThreadRoot(node().id) !== null,
      authored: by !== null,
      "author-card-continued": !bulletless() && continued(),
      "author-card-continues": !bulletless() && continues(),
      "author-run-continued": thread.continued,
      "author-run-continues": thread.continues,
    };
    // Someone else's comment or reply gets its own background, one tint per name
    // (Martin 2026-10-31: the chip alone did not tell them apart).
    if (by !== null) classes[`author-tint-${authorTint(by)}`] = true;
    return classes;
  };
  return { commentSelector, storedAuthor, author, showsAuthor, bulletless, commentClasses };
}

const inlineSurface = (surfaceKey: string) => surfaceKey.startsWith("ref:") || surfaceKey.startsWith("embed:");

/** Body outlines omit annotations at every width. Thread views retain their
 * complete file pre-order, including quoted replies. Membership comes from the
 * covering comment index (`createCommentChildren`), so typing in an ordinary
 * child re-runs none of this (GH #891 review finding 1, round 3). */
export function marginOutline(node: () => CommentNode, _surfaceKey: string, fmt: () => "md" | "org") {
  const showResolved = useResolvedComments();
  // A leaf cannot own comments. Keep just the eligibility comparison until
  // children arrive; the covering index already owns membership for the page.
  const parent = createMemo(() => node()?.children.length ? node().id : null);
  const state = createMemo(() => parent() === null ? null : parentMarginOutline(node, fmt, showResolved));
  return {
    outlineChildren: () => state()?.outlineChildren() ?? node().children,
    marginComments: () => state()?.marginComments() ?? NO_IDS,
    commentChildren: () => state()?.comments() ?? NO_COMMENT_CHILDREN,
    quoteCoverage: () => state()?.quoteCoverage,
  };
}

function parentMarginOutline(node: () => CommentNode, fmt: () => "md" | "org", showResolved: () => boolean) {
  const quoteCoverage = createQuoteCoverage();
  const comments = createCommentChildren(() => node().id);
  const main = () => !!node() && threadRootOf(node().id, fmt()) === null;
  // A body editor that is hand-writing its first quote property must finish
  // that buffer before moving to an annotation surface. Otherwise accepting
  // `tine.quote::` unmounts the editor in the middle of the property value.
  const heldBody = createMemo<string | null>(previous => {
    const n = node();
    if (!n?.children.length) return null;
    const id = editingId();
    if (!id || !n.children.includes(id)) return null;
    return previous === id || !comments().has(id) ? id : null;
  }, null);
  // GH #845: with the margin hidden no surface draws a thread (not even inline).
  const outlineChildren = createMemo(() => {
    const n = node();
    if (!n) return [];
    if (datedThreadRoot(n.id)) return n.children.filter(id => !hasMessageDate(id));
    if (!comments().entries.length) return n.children;
    if (main() || commentMarginHidden()) return n.children.filter(id => id === heldBody() || !comments().has(id));
    // Replies retain file pre-order, even when a hand-nested reply has a quote.
    return n.children;
  }, [], { equals: sameIds });
  const marginComments = createMemo(() => (main() && !commentMarginHidden() ? comments().entries.flatMap(entry => entry.id !== heldBody()
    && (showResolved() || !entry.resolved) ? [entry.id] : []) : []), [], { equals: sameIds });
  return { outlineChildren, marginComments, comments, quoteCoverage };
}

function sameIds(a: readonly string[], b: readonly string[]): boolean {
  return a === b || (a.length === b.length && a.every((id, i) => id === b[i]));
}

/** Body navigation skips annotations through the shared tree API. A margin
 * thread additionally returns to its commented block (up) or the next body
 * block (down); inline thread navigation keeps its existing outline order. */
export function marginNavigation(id: string, scope: OutlineScope | null, surfaceKey: string, fmt: () => "md" | "org") {
  const place = useContext(MarginContext);
  const nav = () => place && (place.thread !== null || (place.api.active() && !inlineSurface(surfaceKey)))
    ? { thread: place.thread, format: fmt() } : null;
  const viewPrev = () => { const n = nav(); return n ? marginPrev(id, scope, n) : outlinePrev(id, scope); };
  const viewNext = () => { const n = nav(); return n ? marginNext(id, scope, n) : outlineNext(id, scope); };
  return { viewPrev, viewNext };
}
