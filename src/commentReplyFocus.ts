import { graphScopedSignal } from "./binding";

// A request survives the editor closing and the wide column mounting. Consumers
// use their own realm's textarea and the originating surface when one is known.
const [replyFocusRequest, setReplyFocusRequest] = graphScopedSignal<{ id: string; surface: string | null; preventScroll?: boolean }>();
export { replyFocusRequest };
export function focusCommentReply(id: string, surface: string | null, preventScroll = false): void { setReplyFocusRequest({ id, surface, preventScroll }); }
export function consumeReplyFocus(): void { setReplyFocusRequest(null); }
