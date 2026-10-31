// What a Block needs from the margin dialogue (vision §3.7), kept out of
// Block.tsx so the component only wires it (the size ratchet, PARITY-CAMPAIGN §3).

import { useContext } from "solid-js";
import type { OutlineScope } from "../document";
import { authorOf, quoteSelectorOf } from "../comments";
import { AUTHOR_TINTS, authorTint, isOwnAuthor } from "../commentIdentity";
import { isCommentId, marginNext, marginPrev } from "../margin";
import { commentsFirst, nextVisible, prevVisible } from "../document";
import { inCommentThread } from "./CommentParts";
import { MarginContext } from "./marginContext";

interface CommentNode { parent: string | null; children: readonly string[] }
type Props = readonly (readonly [string, string])[];

/** Plain functions over the block's facets: an ordinary block pays two property
 * scans and allocates nothing. The thread test walks up the parents rather than
 * providing a context per level (128-level outlines must render). */
export function commentFacts(node: () => CommentNode, properties: () => Props, fmt: () => "md" | "org") {
  const inThread = () => inCommentThread(node().parent, fmt());
  const commentSelector = () => (node().parent !== null ? quoteSelectorOf(properties()) : null);
  // Your own name (Settings → Your name) is "you": null, as for a block without `author::`.
  const author = () => {
    const written = authorOf(properties());
    return written !== null && isOwnAuthor(written) ? null : written;
  };
  const showsAuthor = () => author() !== null || commentSelector() !== null || inThread();
  // Slice 2b: a comment and its replies draw no bullet or fold arrow; the author
  // chip is their handle (drag, block menu).
  const bulletless = () => commentSelector() !== null || inThread();
  const commentClasses = () => {
    const by = author();
    const classes: Record<string, boolean> = {
      "comment-card": commentSelector() !== null && !inThread(),
      "comment-in-thread": bulletless(),
      authored: by !== null,
    };
    // Someone else's comment or reply gets its own background, one tint per name
    // (Martin 2026-10-31: the chip alone did not tell them apart).
    const tint = by !== null && bulletless() ? authorTint(by) : -1;
    for (let i = 0; i < AUTHOR_TINTS; i++) classes[`author-tint-${i}`] = i === tint;
    return classes;
  };
  return { commentSelector, author, showsAuthor, bulletless, commentClasses };
}

const inlineSurface = (surfaceKey: string) => surfaceKey.startsWith("ref:") || surfaceKey.startsWith("embed:");

/** Slice 2: on a page whose comments are drawn in the margin, the main column's
 * outline leaves them out (the count marker stands for them). Inside a margin
 * thread, without a margin, and in reference and embed copies every child is
 * drawn, the comments first, right under their block (commentOrder.ts). */
export function marginOutline(node: () => CommentNode, surfaceKey: string, fmt: () => "md" | "org") {
  const place = useContext(MarginContext);
  const main = () => !!place && place.thread === null && place.api.active() && !inlineSurface(surfaceKey);
  const outlineChildren = () => (main() ? node().children.filter((id) => !isCommentId(id, fmt())) : commentsFirst(node().children));
  const marginComments = () => (main() ? node().children.filter((id) => isCommentId(id, fmt())) : []);
  return { outlineChildren, marginComments };
}

/** With comments drawn in the margin, the main column's arrow keys step over
 * threads, and a thread's ends lead back to the commented block (up) or the
 * block after it (down). */
export function marginNavigation(id: string, scope: OutlineScope | null, surfaceKey: string, fmt: () => "md" | "org") {
  const place = useContext(MarginContext);
  const nav = () => place && (place.thread !== null || (place.api.active() && !inlineSurface(surfaceKey)))
    ? { thread: place.thread, format: fmt() } : null;
  const viewPrev = () => { const n = nav(); return n ? marginPrev(id, scope, n) : prevVisible(id, scope); };
  const viewNext = () => { const n = nav(); return n ? marginNext(id, scope, n) : nextVisible(id, scope); };
  return { viewPrev, viewNext };
}
