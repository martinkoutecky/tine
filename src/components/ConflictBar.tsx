import { For, Show, type JSX } from "solid-js";
import { conflicts, clearConflict } from "../ui";
import { backend } from "../backend";
import { reloadPage, forceSave, canForceSave, pageByName, forgetPage } from "../store";

// Global save-conflict surface. A save is refused (not clobbered) when the file
// changed on disk under us (external edit / Syncthing). Such a page is parked in
// `conflicts` and skipped by every future save batch until resolved — so it MUST
// be surfaced no matter where the page lives (main view, journals feed, sidebar,
// or a query result), or its edits would be silently stuck and lost on close.
export function ConflictBar(): JSX.Element {
  const reload = async (name: string) => {
    const page = pageByName(name);
    // Resolve the file this editor is actually pinned to. Two files can carry
    // one page name (the duplicate-day stray of #21, or same-titled pages in
    // different folders), and resolving by name reaches the backend's CANONICAL
    // owner — which would re-point the tab at a different file and discard the
    // user's edits to this one. Falling back to the name when the pinned file is
    // gone would do the same, so an absent pinned file drops the page instead.
    // A page with no pin (never saved) has only its name to resolve by.
    const dto = page?.path
      ? await backend().getPageByPath(page.path)
      : await backend().getPage(name, page?.kind ?? "page");
    if (dto) {
      reloadPage(dto);
      clearConflict(name);
    } else {
      // The file is gone on disk (deleted/renamed externally). "Use disk version"
      // = accept that: drop the page and its unsaved edits from the store, rather
      // than clearing the conflict and leaving untracked content to be lost silently.
      forgetPage(name);
    }
  };
  const keepMine = async (name: string) => {
    // Only clear the conflict if the overwrite actually landed — otherwise the
    // edit is still unsaved and must stay surfaced.
    if (await forceSave(name)) clearConflict(name);
  };

  return (
    <Show when={conflicts().length > 0}>
      <div class="conflict-stack">
        <For each={conflicts()}>
          {(name) => (
            <div class="conflict-banner">
              <span class="conflict-msg">
                <strong>“{name}” changed outside this editor</strong> (edited elsewhere or synced in). Your
                unsaved changes weren't written.
              </span>
              <span class="conflict-actions">
                <button class="conflict-btn" onClick={() => void reload(name)}>
                  Use current version
                </button>
                <button
                  class="conflict-btn keep"
                  disabled={!canForceSave(name)}
                  title={canForceSave(name)
                    ? "Replace the current version with your retained draft"
                    : "Keep mine is unavailable because the current managed page could not be identified"}
                  onClick={() => void keepMine(name)}
                >
                  Keep mine
                </button>
              </span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
