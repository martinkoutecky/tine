/** "Continue writing below this page": the one implementation behind a page's
 * trailing "+ Add block" target (Page.tsx `PageTypingTarget`) and the
 * `tine://capture` route (deepLinkNavigation.ts). An empty page gets its
 * phantom bullet (`ensureEmptyBlock`, not dirty until the user types);
 * otherwise a fresh root-level block goes after the last root. Returns
 * whether an editor was started. */
import { ensureEmptyBlock, insertOutlineAfter, type FeedPage } from "../document";
import { startEditing } from "../editorController";
import { pushToast } from "../toasts";

export function focusPageTrailing(page: FeedPage | undefined, surface: string | null): boolean {
  if (!page || page.readOnly || page.guide) return false;
  const seeded = ensureEmptyBlock(page.name, { afterProperties: true });
  if (seeded) {
    startEditing(seeded, 0, null, surface);
    return true;
  }
  // GH #158: always add a fresh root-level block (never reuse the trailing empty
  // leaf). Reuse stranded users whose last block is an empty *indented* bullet:
  // clicking could only ever re-focus that indented block, never give them a new
  // unindented last block. Stacking empty last blocks is intentionally allowed.
  const roots = page.roots;
  const id = insertOutlineAfter(roots[roots.length - 1], [{ raw: "", children: [] }]);
  if (id) startEditing(id, 0, null, surface);
  else pushToast("Could not add a block to this page.", "error");
  return !!id;
}
