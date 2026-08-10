import { For, Show, type JSX } from "solid-js";
import { conflicts, clearConflict } from "../ui";
import { backend } from "../backend";
import {
  dropObservation,
  reobserve,
  saveBaselineFor,
  shownObservationFor,
} from "../persistence";
import {
  editorActivationFor,
  forceSave,
  forgetPage,
  pageByName,
  pageInstanceGeneration,
  reloadPage,
} from "../store";

// Global save-conflict surface. A save is refused (not clobbered) when the file
// changed on disk under us (external edit / Syncthing). Such a page is parked in
// `conflicts` and skipped by every future save batch until resolved — so it MUST
// be surfaced no matter where the page lives (main view, journals feed, sidebar,
// or a query result), or its edits would be silently stuck and lost on close.
export function ConflictBar(): JSX.Element {
  const reload = async (name: string) => {
    const page = pageByName(name);
    // "Use disk version" is an authority-answering action, exactly like "Keep
    // mine", so it PRESENTS the observation it was clicked under and lets the
    // backend decide. A locally recorded epoch cannot be trusted here: the
    // raw-watcher path revokes an observation with no page event to react to, so
    // every local value can still compare equal while the authority is already
    // gone. (GH #254 increment 3.)
    const shown = shownObservationFor(name);
    const activation = editorActivationFor(name);
    // Captured AT THE CLICK, so later input can be told apart from what the user
    // was actually looking at when they chose to discard it.
    const generation = pageInstanceGeneration(name);
    // Resolve the file this editor is actually pinned to. Two files can carry
    // one page name (the duplicate-day stray of #21, or same-titled pages in
    // different folders), and resolving by name reaches the backend's CANONICAL
    // owner — which would re-point the tab at a different file and discard the
    // user's edits to this one. Falling back to the name when the pinned file is
    // gone would do the same, so an absent pinned file drops the page instead.
    // A page with no pin (never saved) has only its name to resolve by.
    if (page?.path && shown !== null && activation !== undefined) {
      const outcome = await backend().presentConflictOverride(
        page.path,
        // The episode is { loaded_revision, activation }, so this must name the
        // same baseline the refused save did or the equality refuses the very
        // editor whose banner this is.
        saveBaselineFor(name),
        activation,
        shown,
      );
      if (outcome !== "authorised") {
        // Neither refusal may leave a dead banner. `superseded` means a newer
        // observation is live, so re-observing surfaces it for the user to answer;
        // `withdrawn` means the authority is gone with no successor, which is the
        // state that would otherwise sit dead forever.
        dropObservation(name);
        void reobserve(name);
        return;
      }
    }
    const dto = page?.path
      ? await backend().getPageByPath(page.path)
      : await backend().getPage(name, page?.kind ?? "page");
    // Re-check at the FINAL boundary, not only before the awaited read. The click
    // authorised discarding what was on screen when it was clicked; typing during
    // the await is not that. Typing cancels the whole discard — including the
    // pre-click draft — and the page reverts to ordinary dirty-editor semantics,
    // carried by the re-observing save rather than the ordinary one, which returns
    // before the backend while the page is still conflicted.
    if (pageInstanceGeneration(name) !== generation) {
      dropObservation(name);
      void reobserve(name);
      return;
    }
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
                <strong>“{name}” changed on disk</strong> (edited elsewhere or synced in). Your
                unsaved changes weren't written.
              </span>
              <span class="conflict-actions">
                <button class="conflict-btn" onClick={() => void reload(name)}>
                  Use disk version
                </button>
                <button class="conflict-btn keep" onClick={() => void keepMine(name)}>
                  Keep mine (overwrite)
                </button>
              </span>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}
