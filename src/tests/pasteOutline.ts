import { insertEditorOutline, node } from "../document";
import { BUILTIN_HIDE } from "../editor/properties";
import type { OutlineNode } from "../editor/outline";
import { msg } from "../i18n";

/** What a multi-line paste into block `id` does when the editor's text is unchanged: the editors' own
 *  call (`insertEditorOutline`), which puts the outline in place of an empty block and after any other. */
export function pasteOutlineInto(id: string, nodes: OutlineNode[]): string | null {
  return insertEditorOutline(id, nodes, { raw: node(id).raw, write() {} }, "paste", {
    hide: BUILTIN_HIDE, tooDeep: msg("editor-block-pasted-outline-is-too-deep"), refused: msg("editor-block-pasted-outline-was-refused"),
  });
}
