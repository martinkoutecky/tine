import { createEffect, createSignal, on, onCleanup, Show } from "solid-js";
import type { IndexingProgress } from "../backend";
import { graphEpoch } from "../ui";
import { followIndexingProgress, indexingProgressLabel } from "../indexingProgress";

/** A compact toolbar indicator for graph-sized index work (GH #543): the
 *  launch pass and any later repair in the same graph session.
 *  The app stays usable meanwhile; this only says how long the wait is. */
export function IndexingProgressBar() {
  const [progress, setProgress] = createSignal<IndexingProgress | null>(null);
  createEffect(on(graphEpoch, (epoch) => {
    const stop = new AbortController();
    onCleanup(() => stop.abort());
    void followIndexingProgress(
      epoch,
      (next) => { if (!stop.signal.aborted) setProgress(next); },
      undefined,
      stop.signal,
    );
  }));
  return (
    <Show when={progress()}>
      {(current) => {
        const label = () => indexingProgressLabel(current());
        const fraction = () =>
          current().total > 0 ? Math.min(1, current().done / current().total) : null;
        return (
          <div
            class="indexing-progress"
            role="progressbar"
            aria-label={label()}
            aria-valuemin={0}
            aria-valuemax={current().total > 0 ? current().total : undefined}
            aria-valuenow={current().total > 0 ? current().done : undefined}
            title={label()}
          >
            <span class="indexing-progress-label">{label()}</span>
            <span class="indexing-progress-track">
              <span
                class="indexing-progress-fill"
                classList={{ "indexing-progress-indeterminate": fraction() === null }}
                style={fraction() === null ? undefined : { width: `${(fraction()! * 100).toFixed(1)}%` }}
              />
            </span>
          </div>
        );
      }}
    </Show>
  );
}
