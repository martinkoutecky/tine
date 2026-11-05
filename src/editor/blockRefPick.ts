import { persistBlockRefTarget } from "../document";
import { msg } from "../i18n";
import { pushToast } from "../toasts";
import type { PageKind } from "../types";

export interface BlockRefPickTarget { uuid: string; externalId: string; page: string; kind: PageKind }

export interface BlockRefPickEditor {
  /** The editor still shows the text and trigger the pick was made against. */
  intended: () => boolean;
  /** The editor is still bound to the graph the pick was made in. */
  current: () => boolean;
  /** The completion trigger the pick answered is still open. */
  triggerOpen: () => boolean;
  /** The page of the block being edited, if it is still loaded. */
  sourcePage: () => string | undefined;
  /** Replace the trigger with `text` (the reference). */
  insert: (text: string) => void;
}

/** Accept a `((` / `{{embed ((` block pick: the one path both block editors
 * take. The reference inserted is always the id persistBlockRefTarget stamped
 * on the target, never the pick's runtime key: an id-less target gets a fresh
 * UUID (GH #798), and the CM6 editor's own copy of this path once inserted the
 * runtime key next to it, a reference that resolves to nothing. */
export function acceptBlockRefPick(target: BlockRefPickTarget, editor: BlockRefPickEditor): void {
  let inserted = false;
  void persistBlockRefTarget(target.uuid, target.page, target.kind, undefined, target.externalId, (stampedId) => {
    if (!editor.intended()) return null;
    const sourcePage = editor.sourcePage();
    if (!sourcePage) return null;
    editor.insert(`((${stampedId}))`);
    inserted = true;
    return sourcePage;
  }, editor.intended).then((saved) => {
    if (!saved && editor.current() && (inserted || editor.triggerOpen()))
      pushToast(msg("editor-block-could-not-save-the-block-reference-resolve-the-page-save-and-try-again"), "error");
  }).catch((error) => {
    if (editor.current()) pushToast(msg("editor-block-could-not-save-the-block-reference", { detail: String(error) }), "error");
  });
}
