import { For, Show, type JSX } from "solid-js";
import { conflictReason, conflicts, pageByName, resolveConflict } from "../document";
import { liveConflictForPage } from "../liveConflicts";
import { openPageTarget } from "../routerBridge";

// Global save-conflict surface. The page host refuses to overwrite a file that
// changed on disk under this window's input (external edit / Syncthing) and
// keeps that input's crash-recovery copy; the page stays in `conflicts` until
// resolved, so it MUST be surfaced wherever the page lives (main view, journals
// feed, sidebar, or a query result). A page with a file is resolved in the
// in-page review, block by block, as in master: the bar offers "Review" and
// never a blind "Keep mine (overwrite)" for it. A page with no file of its own
// yet (a file appeared under its name), or whose file was deleted on disk
// (nothing to review against or open, GH #541), offers the two whole-page
// choices: Use disk version accepts the deletion, Keep mine recreates the file.
export function ConflictBar(): JSX.Element {
  return (
    <Show when={conflicts().length > 0}>
      <div class="conflict-stack">
        <For each={conflicts()}>
          {(name) => {
            const live = () => { const page = pageByName(name); return page?.id ? liveConflictForPage(name, page.id) : undefined; };
            return (
            <Show when={!live()} fallback={
              <div class="conflict-banner">
                <span class="conflict-msg"><strong>“{name}” changed on disk while you were editing it.</strong> Your edits are kept; review them against the disk version on the page.</span>
                <span class="conflict-actions">
                  <button class="conflict-btn" onClick={() => { const c = live(); if (c) openPageTarget({ name: c.page_name, pageKind: c.kind, path: c.page_path }); }}>Review</button>
                </span>
              </div>
            }>
            <div class="conflict-banner">
              <span class="conflict-msg">
                <strong>“{name}” {conflictReason(name)?.observedRev === null ? "was deleted on disk" : "changed on disk"}</strong>. Your unsaved changes weren't written.
              </span>
              <span class="conflict-actions">
                <button class="conflict-btn" onClick={() => void resolveConflict(name, "disk")}>
                  Use disk version
                </button>
                <button class="conflict-btn keep" onClick={() => void resolveConflict(name, "mine")}>
                  Keep mine (overwrite)
                </button>
              </span>
            </div>
            </Show>
          );}}
        </For>
      </div>
    </Show>
  );
}
