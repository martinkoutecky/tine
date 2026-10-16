import { t, msg } from "../i18n";
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
import { resolveBlockRef, node as docNode, type blockRef } from "../document";
import { loadSourcePage } from "../sourcePage";
import { bindingOwner } from "../owned";
import { leadingMarker, taskCheckboxState } from "../markers";
import { pushToast } from "../toasts";
import { formatForBlockId, toggleBlockCheckbox } from "./blockParts";
import { noticeChangedResults, resolveResultRow, type ResultRow } from "./queryResultIdentity";

/** A durable reference to a block plus its exact owner page. */
export type LoadedBlockRef = ReturnType<typeof blockRef>;

export type TaskToggleOutcome = "toggled" | "stale" | "missing" | "refused";

/** Toggle the task checkbox of the block `ref` names, on its source page.
 * `expectChecked` is the state the user clicked: when the live source no longer
 * shows it (another view, device or editor changed the task meanwhile), nothing
 * is written and the rendered state catches up instead, so a click never
 * acts on a state the user did not see. Cost: one page read when the source page
 * is not loaded, plus O(page blocks) for the load and the reference resolution. */
export async function toggleTaskAtSource(ref: LoadedBlockRef, expectChecked: boolean, result?: ResultRow): Promise<TaskToggleOutcome> {
  const owner = bindingOwner();
  if (!(await loadSourcePage(ref, "The task was not changed."))) return "refused";
  if (!owner()) return "stale";
  const source = result ? resolveResultRow(result) : null;
  if (result && !source) { noticeChangedResults(1); return "stale"; }
  const id = source?.id ?? resolveBlockRef(ref);
  if (id === null || !docNode(id)) {
    pushToast(msg("macro-source-task-checkbox-the-task-is-no-longer-on-so-it-was-not", { value1: ref.page }), "error");
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
  result?: ResultRow;
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
          title={state() === true ? t("macro-source-task-checkbox-mark-undone") : t("macro-source-task-checkbox-mark-done")}
          onMouseDown={(e) => {
            e.stopPropagation();
            e.preventDefault();
            if (e.button !== 0) return;
            const ref = props.target();
            const checked = state();
            if (!ref || checked === null) return;
            void toggleTaskAtSource(ref, checked, props.result).catch(() => {
              pushToast(msg("macro-source-task-checkbox-couldn-t-load-the-task-s-page-so-the-task"), "error");
            });
          }}
          // Keep this a direct touch-adjustment candidate like the source box.
          on:click={(e) => { e.stopPropagation(); e.preventDefault(); }}
          onAuxClick={(e) => { e.stopPropagation(); e.preventDefault(); }}
        />
    </Show>
  );
}
