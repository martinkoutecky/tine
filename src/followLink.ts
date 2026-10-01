// GH #274: open the link at the caret without reaching for the mouse.
//
// OG bindings this reproduces (`frontend/modules/shortcut/config.cljs`):
//   :editor/follow-link          mod+o        -> editor-handler/follow-link-under-cursor!
//   :editor/open-link-in-sidebar mod+shift+o  -> editor-handler/open-link-in-sidebar!
//
// The "which link" rule lives in `editor/nearestLink.ts`, transcribed from OG's
// `thingatpt.cljs`. This module is only the dispatch: what to DO with what was
// found, and how to read the caret out of the live editor.

import { nearestLink, type NearestLink } from "./editor/nearestLink";
import { node as docNode, pageByName } from "./document";
import { editingId } from "./editorController";
import type { Format } from "./render/ast";
import { openPage, openPageAtBlock } from "./router";
import { openPageInSidebar, openBlockInSidebar } from "./ui";
import { pushToast } from "./toasts";
import { backend } from "./backend";
import { blockRefTarget, resolveBlockBatched } from "./resolveBatch";
import { graphOwner, ownedWhen, readOwned } from "./owned";

type CaretContext = { text: string; caret: number; format: Format };

/** The focused block editor's text, caret and source format, or null when not
 *  editing. The format is the editing block's page format: Org and Markdown
 *  disagree on what is literal (e.g. `~[[x]]~` is Org code). */
function caretContext(): CaretContext | null {
  if (typeof document === "undefined") return null;
  const active = document.activeElement;
  if (!(active instanceof HTMLTextAreaElement)) return null;
  const id = editingId();
  const node = id ? docNode(id) : undefined;
  const format: Format = node && pageByName(node.page)?.format === "org" ? "org" : "md";
  return { text: active.value, caret: active.selectionStart ?? 0, format };
}

export interface FollowLinkDeps {
  /** Source of the editor text, caret and format; defaults to the focused textarea. */
  read?(): CaretContext | null;
}

/** `mod+o`: follow the link nearest the caret in the block being edited.
 *
 *  Contract: returns false (and does nothing) when nothing is being edited or
 *  the text holds no link; otherwise dispatches and returns true. URLs open
 *  externally (a failure is toasted); page refs and tags navigate in place;
 *  a bare `((uuid))` resolves its owner asynchronously through the shared
 *  block-ref resolver and toasts when the block is not found. */
export function followLinkUnderCaret(deps: FollowLinkDeps = {}): boolean {
  const context = (deps.read ?? caretContext)();
  if (!context) return false;
  const link = nearestLink(context.text, context.caret, { includeUrls: true, format: context.format });
  if (!link) return false;
  return dispatch(link, "here");
}

/** `mod+shift+o`: as `followLinkUnderCaret`, but opens in the right sidebar
 *  and never considers URLs — a URL has no sidebar representation, which is
 *  also why OG's sidebar command omits the url pattern. */
export function openLinkUnderCaretInSidebar(deps: FollowLinkDeps = {}): boolean {
  const context = (deps.read ?? caretContext)();
  if (!context) return false;
  const link = nearestLink(context.text, context.caret, { format: context.format });
  if (!link) return false;
  return dispatch(link, "sidebar");
}

function dispatch(link: NearestLink, where: "here" | "sidebar"): boolean {
  if (link.kind === "url") {
    // Not graph-scoped: opening a URL stays meaningful across graph switches,
    // so the owner is always live and a failure is always surfaced (I-9).
    void readOwned(ownedWhen(), backend().openExternal(link.value)).catch(() => {
      pushToast(`Couldn't open ${link.value}`, "error");
    });
    return true;
  }
  if (link.kind === "block") {
    // Raw `((uuid))` carries no page: resolve it the same way a rendered block
    // ref does (working set first, then the backend), so a block outside the
    // loaded pages is still found — master gave up there.
    const uuid = link.value;
    void readOwned(graphOwner(), resolveBlockBatched(uuid)).then((result) => {
      if (result.kind === "stale") return;
      const g = result.value;
      if (!g) { pushToast("Couldn't find the referenced block", "error"); return; }
      const ref = blockRefTarget(uuid, g);
      if (where === "sidebar") openBlockInSidebar(ref);
      else openPageAtBlock({ name: ref.page, pageKind: ref.pageKind, block: ref.uuid, ...(ref.path ? { path: ref.path } : {}) });
    });
    return true;
  }
  // Page and tag are the same destination; OG strips the `#` and routes both
  // through the page name.
  if (where === "sidebar") openPageInSidebar(link.value);
  else openPage(link.value);
  return true;
}
