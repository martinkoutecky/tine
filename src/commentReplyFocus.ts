import { graphScopedSignal } from "./binding";

// A request survives the editor closing and the wide column mounting. Consumers
// use their own realm's textarea and the originating surface when one is known.
const [replyFocusRequest, setReplyFocusRequest] = graphScopedSignal<{ id: string; surface: string | null; preventScroll?: boolean; reveal?: boolean }>();
export { replyFocusRequest };
/** `reveal`: an explicit reply (Comment on a passage that has a thread) expands a folded
 *  thread to reach its reply box; review navigation focuses the folded card instead. */
export function focusCommentReply(id: string, surface: string | null, preventScroll = false, reveal = false): void { setReplyFocusRequest({ id, surface, preventScroll, reveal }); }
export function consumeReplyFocus(): void { setReplyFocusRequest(null); }
