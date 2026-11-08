// What a Block needs from the margin dialogue (vision §3.7), kept out of
// Block.tsx so the component only wires it (the size ratchet, PARITY-CAMPAIGN §3).

import { createMemo, useContext } from "solid-js";
import type { OutlineScope } from "../document";
import { authorOf, quoteSelectorOf } from "../comments";
import { AUTHOR_TINTS, authorTint, isOwnAuthor } from "../commentIdentity";
import { isCommentId, marginNext, marginPrev, threadRootOf } from "../margin";
import { nextVisible, prevVisible, node as importNode } from "../document";
import { inCommentThread } from "./CommentParts";
import { MarginContext } from "./marginContext";
import { commentMarginHidden } from "../commentMarginPref";
import { facetsOf } from "../render/facets";
import { authorRunContinues, type AuthorNeighbors } from "./authorRuns";
import { editingId } from "../editorController";
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
 * complete file pre-order, including quoted replies. */
export function marginOutline(node: () => CommentNode, _surfaceKey: string, fmt: () => "md" | "org") {
  const main = () => !!node() && threadRootOf(node().id, fmt()) === null;
  // A body editor that is hand-writing its first quote property must finish
  // that buffer before moving to an annotation surface. Otherwise accepting
  // `tine.quote::` unmounts the editor in the middle of the property value.
  const heldBody = createMemo<string | null>(previous => {
    const n = node();
    if (!n?.children.length) return null;
    const id = editingId();
    if (!id || !n.children.includes(id)) return null;
    return previous === id || !isCommentId(id, fmt()) ? id : null;
  }, null);
  // GH #845: with the margin hidden no surface draws a thread (not even inline).
  const outlineChildren = () => {
    const n = node();
    if (!n) return [];
    if (main() || commentMarginHidden()) return n.children.filter(id => id === heldBody() || !isCommentId(id, fmt()));
    // Replies retain file pre-order, even when a hand-nested reply has a quote.
    return n.children;
  };
  const marginComments = () => (main() && !commentMarginHidden() ? node().children.filter((id) => id !== heldBody() && isCommentId(id, fmt())
    && !facetsOf(importNode(id)?.raw ?? "", fmt()).properties.some(([key, value]) => key.toLowerCase() === "tine.resolved" && value.trim().toLowerCase() === "true")) : []);
  return { outlineChildren, marginComments };
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
