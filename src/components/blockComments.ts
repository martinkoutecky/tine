// What a Block needs from the margin dialogue (vision §3.7), kept out of
// Block.tsx so the component only wires it (the size ratchet, PARITY-CAMPAIGN §3).

import { useContext } from "solid-js";
import type { OutlineScope } from "../document";
import { authorOf, quoteSelectorOf } from "../comments";
import { isOwnAuthor } from "../commentIdentity";
import { isCommentId, marginNext, marginPrev } from "../margin";
import { nextVisible, prevVisible } from "../document";
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
  const commentClasses = () => ({
    "comment-card": commentSelector() !== null && !inThread(),
    "comment-in-thread": bulletless(),
    authored: author() !== null,
  });
  return { commentSelector, author, showsAuthor, bulletless, commentClasses };
}

const inlineSurface = (surfaceKey: string) => surfaceKey.startsWith("ref:") || surfaceKey.startsWith("embed:");

/** Slice 2: on a page whose comments are drawn in the margin, the main column's
 * outline leaves them out (the count marker stands for them). Inside a margin
 * thread, without a margin, and in reference and embed copies every child is drawn. */
export function marginOutline(node: () => CommentNode, surfaceKey: string, fmt: () => "md" | "org") {
  const place = useContext(MarginContext);
  const main = () => !!place && place.thread === null && place.api.active() && !inlineSurface(surfaceKey);
  const outlineChildren = () => (main() ? node().children.filter((id) => !isCommentId(id, fmt())) : node().children);
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
