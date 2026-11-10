// What a Block needs from the margin dialogue (vision §3.7), kept out of
// Block.tsx so the component only wires it (the size ratchet, PARITY-CAMPAIGN §3).

import { createMemo, useContext } from "solid-js";
import type { OutlineScope } from "../document";
import { authorOf, quoteSelectorOf } from "../comments";
import { AUTHOR_TINTS, authorTint, isOwnAuthor } from "../commentIdentity";
import { marginNext, marginPrev, threadRootOf } from "../margin";
import { nextVisible, prevVisible } from "../document";
import { inCommentThread } from "./CommentParts";
import { MarginContext } from "./marginContext";
import { commentMarginHidden } from "../commentMarginPref";
import { authorRunContinues, type AuthorNeighbors } from "./authorRuns";
import { threadResolved } from "../commentResolution";
import { useResolvedComments } from "./CommentResolutionScope";
import { editingId } from "../editorController";
import { createCommentChildren } from "../commentIndex";
import { createQuoteCoverage } from "../render/quoteHighlight";
export { commentEditorHide } from "../editor/commentProperties";

interface CommentNode { id: string; raw: string; parent: string | null; children: readonly string[] }
type Props = readonly (readonly [string, string])[];

/** Local facets and a memo for the adjacent authorship pairs. The thread test
 * walks up the parents rather than providing a context per level (128-level
 * outlines must render). Unauthored blocks do no neighbour comparisons. */
export function commentFacts(node: () => CommentNode, properties: () => Props, fmt: () => "md" | "org", run?: { id: () => string; neighbours: () => AuthorNeighbors | undefined }) {
  const inThread = () => inCommentThread(node().parent, fmt());
  const commentSelector = () => (node().parent !== null ? quoteSelectorOf(properties()) : null);
  // Your own name (Settings → Your name) is "you": null, as for a block without `tine.author::`.
  const author = () => {
    const written = authorOf(properties());
    return written !== null && isOwnAuthor(written) ? null : written;
  };
  // Keep pair comparisons independent of unrelated row-chrome invalidations.
  // Only the list IDs and these three nodes' local fields invalidate this memo.
  const joins = createMemo(() => {
    // Thread presentation is gated below. Do not read ancestors here: their
    // raw text would make a parent's keystroke recompute every descendant run.
    if (author() === null || commentSelector() !== null) return [false, false];
    const neighbours = run?.neighbours()?.();
    return [authorRunContinues(neighbours?.[0], run?.id(), fmt()), authorRunContinues(run?.id(), neighbours?.[1], fmt())];
  });
  const continued = () => joins()[0];
  const continues = () => joins()[1];
  const showsAuthor = () => (author() !== null && !continued()) || commentSelector() !== null || inThread();
  // Comments and their replies draw no bullet; the chip keeps the block menu.
  const bulletless = () => commentSelector() !== null || inThread();
  const commentClasses = () => {
    const by = author();
    const classes: Record<string, boolean> = {
      "comment-card": commentSelector() !== null && !inThread(),
      "comment-resolved": commentSelector() !== null && !inThread() && threadResolved(node().id),
      "comment-in-thread": bulletless(),
      authored: by !== null,
      "author-run-continued": !bulletless() && continued(),
      "author-run-continues": !bulletless() && continues(),
    };
    // Someone else's comment or reply gets its own background, one tint per name
    // (Martin 2026-10-31: the chip alone did not tell them apart).
    const tint = by !== null ? authorTint(by) : -1;
    for (let i = 0; i < AUTHOR_TINTS; i++) classes[`author-tint-${i}`] = i === tint;
    return classes;
  };
  return { commentSelector, author, showsAuthor, bulletless, commentClasses };
}

const inlineSurface = (surfaceKey: string) => surfaceKey.startsWith("ref:") || surfaceKey.startsWith("embed:");

/** Body outlines omit annotations at every width. Thread views retain their
 * complete file pre-order, including quoted replies. Membership comes from the
 * covering comment index (`createCommentChildren`), so typing in an ordinary
 * child re-runs none of this (GH #891 review finding 1, round 3). */
export function marginOutline(node: () => CommentNode, _surfaceKey: string, fmt: () => "md" | "org") {
  const showResolved = useResolvedComments();
  const comments = createCommentChildren(() => node()?.id ?? null);
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
    if (!comments().entries.length) return n.children;
    if (main() || commentMarginHidden()) return n.children.filter(id => id === heldBody() || !comments().has(id));
    // Replies retain file pre-order, even when a hand-nested reply has a quote.
    return n.children;
  }, [], { equals: sameIds });
  const marginComments = createMemo(() => (main() && !commentMarginHidden() ? comments().entries.flatMap(entry => entry.id !== heldBody()
    && (showResolved() || !entry.resolved) ? [entry.id] : []) : []), [], { equals: sameIds });
  // What this instance's renderer highlights; outlives the rendering while the
  // block is edited, so the entry point keeps its last answer (review round 2).
  const quoteCoverage = createQuoteCoverage();
  return { outlineChildren, marginComments, commentChildren: comments, quoteCoverage };
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
  const viewPrev = () => { const n = nav(); return n ? marginPrev(id, scope, n) : prevVisible(id, scope); };
  const viewNext = () => { const n = nav(); return n ? marginNext(id, scope, n) : nextVisible(id, scope); };
  return { viewPrev, viewNext };
}
