import type { Edit } from "../editor/format";
import { youtubeTimestampMacroFor } from "./MediaMacros";
export { youtubeTimestampMacroFor } from "./MediaMacros";

/** OG commands.cljs insert-youtube-timestamp: macro plus space, or no edit
 * without a ready player. The slash command uses the same player selector. */
export function insertYoutubeTimestamp(editor: HTMLTextAreaElement, apply: (edit: Edit) => void): void {
  const macro = youtubeTimestampMacroFor(editor);
  if (!macro) return;
  const inserted = `${macro} `;
  const start = editor.selectionStart;
  apply({ text: editor.value.slice(0, start) + inserted + editor.value.slice(editor.selectionEnd), start: start + inserted.length, end: start + inserted.length });
}
