import { finishPageHeaderEdit } from "../document";
import type { Format } from "../render/ast";
import { leadingMarker } from "./marker";
import { normalizePlanning } from "./planning";

/** Retain the accepted task state through incomplete typed markers. setRaw
 * owns the clock transition and always receives the current reattached history. */
export function editorMarkerBaseline(initial: string, format: Format): (raw: string) => string {
  let baseline = initial;
  return raw => {
    if (leadingMarker(raw, format)) baseline = raw;
    return baseline;
  };
}

/** Normalize only a real exit, never a literal body or an occurrence handoff.
 * Header validation and cleanup use the same document boundary in both editors. */
export function commitBlockEditorExit(ctx: {
  id: string;
  text: string;
  format: Format;
  literal: boolean;
  handoff: boolean;
  commit(text: string): void;
}): void {
  ctx.commit(ctx.literal || ctx.handoff ? ctx.text : normalizePlanning(ctx.text, ctx.format));
  if (!ctx.handoff) finishPageHeaderEdit(ctx.id);
}
