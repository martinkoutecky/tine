// The visible half of "changed since you last looked" (vision 9a, ADR 0073):
// the routed page's tracker, the context its block rows read, and the header
// line with Mark seen. A page without a baseline gets no tracker, no header
// and no row class: it renders exactly as before.
import { Show, createContext, createEffect, createMemo, onCleanup, useContext, type Accessor, type JSX } from "solid-js";
import { createSeenTracker, type SeenTracker } from "./tracker";
import { loadSeenBaseline, markPageSeen, seenBaselineFor } from "./baseline";
import "../styles/seen.css";

/** Which rows are changed on the surface that tracks them; absent elsewhere. */
export const SeenContext = createContext<((id: string) => boolean) | undefined>(undefined);

/** The `.block-main` classes a row takes from the tracking surface above it.
 *  A row outside a tracking surface (journal feed, sidebar, zoom, references)
 *  reads no context and gets nothing. */
export function seenRowClasses(id: string): Record<string, boolean> {
  const changed = useContext(SeenContext);
  return changed ? { "seen-changed": changed(id) } : {};
}

export interface PageSeen {
  tracker: Accessor<SeenTracker | null>;
  changed: (id: string) => boolean;
}

/** Track `pageName()` while it is non-null: load its baseline once, and compare
 *  its blocks while it has one. Disposed with the calling component. */
export function usePageSeen(pageName: Accessor<string | null>): PageSeen {
  createEffect(() => {
    const name = pageName();
    if (name) loadSeenBaseline(name);
  });
  const baseline = createMemo(() => {
    const name = pageName();
    return name ? seenBaselineFor(name) ?? null : null;
  });
  const tracker = createMemo<SeenTracker | null>(() => {
    const name = pageName();
    const current = baseline();
    if (!name || !current) return null;
    const live = createSeenTracker(name, () => current);
    onCleanup(() => live.dispose());
    return live;
  });
  return { tracker, changed: (id) => tracker()?.changed(id) ?? false };
}

/** "N changes since you last looked" + Mark seen, shown only while a tracked
 *  page has changes. */
export function SeenHeader(props: { seen: PageSeen; pageName: string }): JSX.Element {
  const count = () => props.seen.tracker()?.count() ?? 0;
  return (
    <Show when={count() > 0}>
      <div class="seen-header" role="status" data-seen-header>
        <span class="seen-header-text">
          {count()} {count() === 1 ? "change" : "changes"} since you last looked
        </span>
        <button type="button" class="settings-btn seen-header-mark" onClick={() => void markPageSeen(props.pageName)}>
          Mark seen
        </button>
      </div>
    </Show>
  );
}

/** Wrap a tracking surface's rows. */
export function SeenRows(props: { seen: PageSeen; children: JSX.Element }): JSX.Element {
  return <SeenContext.Provider value={props.seen.changed}>{props.children}</SeenContext.Provider>;
}
