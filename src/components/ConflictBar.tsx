import { For, Show, type JSX } from "solid-js";
import { conflicts, clearConflict } from "../ui";
import { backend } from "../backend";
import { reloadPage, forceSave, pageByName } from "../store";

// Global save-conflict surface. A save is refused (not clobbered) when the file
// changed on disk under us (external edit / Syncthing). Such a page is parked in
// `conflicts` and skipped by every future save batch until resolved — so it MUST
// be surfaced no matter where the page lives (main view, journals feed, sidebar,
// or a query result), or its edits would be silently stuck and lost on close.
export function ConflictBar(): JSX.Element {
  const reload = async (name: string) => {
    const kind = pageByName(name)?.kind ?? "page";
    const dto = await backend().getPage(name, kind);
    if (dto) reloadPage(dto);
    clearConflict(name);
  };
  const keepMine = async (name: string) => {
    await forceSave(name);
    clearConflict(name);
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
