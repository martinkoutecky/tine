import { node as docNode, pageByName, persistBlockRefTarget } from "../document";
import { blockDtoExternalId } from "../blockIdentity";
import { blockFirstLine } from "../components/blockParts";
import { msg } from "../i18n";
import { pushToast } from "../toasts";
import type { PageKind, RefGroup } from "../types";
import type { AcItem } from "./completionItems";

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

/** OG `get-matched-blocks` walks at most 99 parents. */
const PICKER_ANCESTOR_LIMIT = 99;

/** The `((` picker's rows for a block search, the one list both block editors
 * show. Like OG `get-matched-blocks`, it never offers the block being edited
 * or any of its ancestors: the edited block always matches its own `((query`
 * once autosaved, and picking it (or a block containing it) makes a reference
 * cycle. */
export function blockPickItems(groups: RefGroup[], editingId: string): AcItem[] {
  const excluded = new Set([editingId]);
  let parent = docNode(editingId)?.parent ?? null;
  while (parent !== null && excluded.size <= PICKER_ANCESTOR_LIMIT && !excluded.has(parent)) {
    excluded.add(parent);
    parent = docNode(parent)?.parent ?? null;
  }
  const items: AcItem[] = [];
  for (const g of groups) {
    for (const b of g.blocks) {
      if (excluded.has(b.id)) continue;
      items.push({
        label: blockFirstLine(b.raw, pageByName(g.page)?.format === "org" ? "org" : "md") || g.page,
        sub: g.page,
        blockRef: { uuid: b.id, externalId: blockDtoExternalId(b), page: g.page, kind: g.kind },
      });
    }
  }
  return items;
}
