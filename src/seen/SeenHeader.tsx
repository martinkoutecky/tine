// The visible half of "changed since you last looked" (ADR 0079): the routed
// page's context (read by its block rows), the header line, and the marker a
// removed block leaves. A page outside a tracking surface (journal feed,
// sidebar, zoom, references) reads no context and renders exactly as before.
import { For, Show, createContext, createSignal, useContext, type JSX } from "solid-js";
import { t } from "../i18n";
import type { PageSeen } from "./pageSeen";
import { markPageSeenNow } from "./seenState";
import "../styles/seen.css";

const SeenContext = createContext<PageSeen | undefined>(undefined);

/** Wrap a tracking surface's rows. */
export function SeenRows(props: { seen: PageSeen; children: JSX.Element }): JSX.Element {
  return <SeenContext.Provider value={props.seen}>{props.children}</SeenContext.Provider>;
}

/** A row's seen facts, read once at mount. Absent outside a tracking surface. */
export function useSeenRow(id: string): { changed: () => boolean } {
  const seen = useContext(SeenContext);
  return { changed: () => seen?.changed(id) ?? false };
}

function RemovedMarker(props: { text: string; child: boolean }): JSX.Element {
  const [open, setOpen] = createSignal(false);
  return (
    <div class="seen-removed" classList={{ "seen-removed-child": props.child }} data-seen-removed>
      <button type="button" class="seen-removed-toggle" aria-expanded={open()} onClick={() => setOpen(!open())}>
        <span class="seen-removed-caret" aria-hidden="true">{open() ? "▾" : "▸"}</span>
        {t("seen-removed-block")}
      </button>
      <Show when={open()}>
        <pre class="seen-removed-text">{props.text}</pre>
      </Show>
    </div>
  );
}

/** Removed blocks drawn after block `after`, or first inside `first` (a block
 *  id, or null for the page's own list). Renders nothing outside a tracking surface. */
export function SeenRemoved(props: { after?: string; first?: string | null }): JSX.Element {
  const seen = useContext(SeenContext);
  if (!seen) return null;
  const marks = () => props.after !== undefined ? seen.removedAfter(props.after) : seen.removedFirst(props.first ?? null);
  return <For each={marks()}>{(mark) => <RemovedMarker text={mark.text} child={props.after === undefined} />}</For>;
}

/** "N changes since you last looked" + Mark page seen, only while N > 0. */
export function SeenHeader(props: { seen: PageSeen; pageName: string }): JSX.Element {
  const notes = () => [
    props.seen.identityKnown() ? "" : t("seen-note-identity-unknown"),
    props.seen.unattributed() ? t("seen-note-unattributed") : "",
    props.seen.reason() ?? "",
  ].filter(Boolean).join(" ");
  return (
    <Show when={props.seen.count() > 0}>
      <div class="seen-header" role="status" data-seen-header>
        <span class="seen-header-text" classList={{ "has-note": !!notes() }} title={notes() || undefined}>
          {t("seen-header-count", { count: props.seen.count() })}
        </span>
        <button type="button" class="settings-btn seen-header-mark" onClick={() => void markPageSeenNow(props.pageName, false)}>
          {t("seen-mark-page")}
        </button>
        <Show when={props.seen.unattributed()}>
          <div class="seen-header-muted" data-seen-unattributed>{t("seen-note-unattributed-line")}</div>
        </Show>
      </div>
    </Show>
  );
}
