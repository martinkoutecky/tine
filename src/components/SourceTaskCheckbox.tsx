/** The task checkbox of a block shown AWAY from its source outline: an inline
 * `((block reference))`, a not-yet-hydrated reference/query row, a hover peek
 * (GH #518). OG renders the same `block-checkbox` inside a block reference
 * (og-1.0.0 frontend/components/block.cljs `block-reference` -> `block-content`
 * with `:block-ref? true` -> `build-block-title` -> `block-checkbox`, whose
 * `on-change` calls `editor-handler/check`/`uncheck` on the referenced block).
 *
 * This adds no write of its own. A toggle loads the source page into the
 * working set when it is not there yet (`ensurePageLoaded`, a read that never
 * clobbers a loaded page's unsaved edits), resolves the durable reference to
 * the live node (`resolveBlockRef`), and then calls `toggleBlockCheckbox`, the
 * exact function the source block's own checkbox calls (`setRaw`: writability
 * check, undo entry, base-revision-guarded save of that page). */
import { Show, type JSX } from "solid-js";
import { backend } from "../backend";
import { ensurePageLoaded, pageByName, reportPageLoadRefusal, resolveBlockRef, node as docNode } from "../document";
import type { LoadedBlockRef } from "../document/edits/identity";
import { graphOwner, readOwned } from "../owned";
import { leadingMarker, taskCheckboxState } from "../markers";
import { pushToast } from "../toasts";
import { formatForBlockId, toggleBlockCheckbox } from "./blockParts";

export type TaskToggleOutcome = "toggled" | "stale" | "missing" | "refused";

/** Toggle the task checkbox of the block `ref` names, on its source page.
 * `expectChecked` is the state the user clicked: when the live source no longer
 * shows it (another view, device or editor changed the task meanwhile), nothing
 * is written and the rendered state catches up instead, so a click never
 * acts on a state the user did not see. Cost: one page read when the source page
 * is not loaded, plus O(page blocks) for the load and the reference resolution. */
export async function toggleTaskAtSource(ref: LoadedBlockRef, expectChecked: boolean): Promise<TaskToggleOutcome> {
  const owner = graphOwner();
  const loaded = pageByName(ref.page);
  if (!loaded || loaded.kind !== ref.pageKind || (ref.path !== undefined && loaded.id !== ref.path)) {
    const result = await readOwned(owner, ref.path ? backend().getPageByPath(ref.path) : backend().getPage(ref.page, ref.pageKind));
    if (result.kind === "stale") return "stale";
    if (!result.value) {
      pushToast(`"${ref.page}" is no longer there, so the task was not changed.`, "error");
      return "missing";
    }
    const refusal = ensurePageLoaded(result.value);
    if (refusal) {
      reportPageLoadRefusal(refusal, "The task was not changed.");
      return "refused";
    }
  }
  const id = resolveBlockRef(ref);
  if (id === null || !docNode(id)) {
    pushToast(`The task is no longer on "${ref.page}", so it was not changed.`, "error");
    return "missing";
  }
  if (taskCheckboxState(leadingMarker(docNode(id).raw, formatForBlockId(id))) !== expectChecked) return "stale";
  toggleBlockCheckbox(id);
  return "toggled";
}

/** The checkbox element, styled like the source block's (`Rendered.tsx`).
 * Renders nothing for a marker with no checkbox (CANCELED, none). `target`
 * is read at click time; null (an unresolved reference) makes the box inert.
 * Mouse-down toggles and swallows both mouse-down and the following click, so
 * the surrounding reference never navigates or enters an editor on the way. */
export function SourceTaskCheckbox(props: {
  marker: string | null | undefined;
  target: () => LoadedBlockRef | null;
  style?: JSX.CSSProperties;
}): JSX.Element {
  const state = () => taskCheckboxState(props.marker);
  return (
    <Show when={state() !== null}>
        <span
          class="block-task-checkbox"
          classList={{ checked: state() === true }}
          style={props.style}
          role="checkbox"
          aria-checked={state() === true}
          title={state() === true ? "Mark undone" : "Mark done"}
          onMouseDown={(e) => {
            e.stopPropagation();
            e.preventDefault();
            if (e.button !== 0) return;
            const ref = props.target();
            const checked = state();
            if (!ref || checked === null) return;
            void toggleTaskAtSource(ref, checked).catch(() => {
              pushToast("Couldn't load the task's page, so the task was not changed.", "error");
            });
          }}
          onClick={(e) => { e.stopPropagation(); e.preventDefault(); }}
          onAuxClick={(e) => { e.stopPropagation(); e.preventDefault(); }}
        />
    </Show>
  );
}
