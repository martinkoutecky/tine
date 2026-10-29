// The visible half of "changed since you last looked" (ADR 0079): the routed
// page's context (read by its block rows), the header line, and the marker a
// removed block leaves. A page outside a tracking surface (journal feed,
// sidebar, zoom, references) reads no context and renders exactly as before.
import { For, Show, createContext, createSignal, onMount, onCleanup, useContext, type JSX } from "solid-js";
import { newResizeObserver } from "../windowRealm";
import { t } from "../i18n";
import type { PageSeen } from "./pageSeen";
import { markPageSeenNow } from "./seenState";
import { registerSeenStepper } from "./stepper";
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

function RemovedMarker(props: { text: string; child: boolean; item: string }): JSX.Element {
  const [open, setOpen] = createSignal(false);
  return (
    <div class="seen-removed" classList={{ "seen-removed-child": props.child }} data-seen-removed data-seen-item={props.item}>
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
  // The n-th marker of its anchor: the key Previous / Next change finds it by (steps.ts).
  const anchor = () => props.after !== undefined ? `a:${props.after}` : `f:${props.first ?? ""}`;
  return <For each={marks()}>{(mark, n) => <RemovedMarker text={mark.text} child={props.after === undefined} item={`${anchor()}:${n()}`} />}</For>;
}

/** Mount with the visible notice, including when its answer arrives later.
 * Reserve its actual wrapped height for scrollIntoView and caret reveals. */
function StickyNotice(props: { children: JSX.Element }): JSX.Element {
  let host!: HTMLDivElement;
  onMount(() => {
    const scroller = host.closest<HTMLElement>(".main-content, .right-sidebar");
    if (!scroller) return;
    const previous = scroller.style.getPropertyValue("--seen-header-height");
    const measure = () => {
      scroller.style.setProperty("--seen-header-height", `${host.getBoundingClientRect().height + 8}px`);
      scroller.dispatchEvent(new Event("seen-notice-resize"));
    };
    measure();
    const observer = newResizeObserver(host, measure);
    observer?.observe(host);
    onCleanup(() => {
      observer?.disconnect();
      if (previous) scroller.style.setProperty("--seen-header-height", previous);
      else scroller.style.removeProperty("--seen-header-height");
      scroller.dispatchEvent(new Event("seen-notice-resize"));
    });
  });
  return <div ref={host} class="seen-header" role="status" data-seen-header>{props.children}</div>;
}

/** Previous / Next change and "3 of 12". Also offers the same steps to the
 *  command palette while this notice is on screen. */
function SeenSteps(props: { seen: PageSeen; pageName: string }): JSX.Element {
  let group!: HTMLSpanElement;
  const scroller = () => group.closest<HTMLElement>(".main-content, .right-sidebar");
  const go = (direction: 1 | -1) => { const host = scroller(); if (host) void props.seen.step(direction, host); };
  onMount(() => onCleanup(registerSeenStepper(props.pageName, go)));
  return (
    <span class="seen-header-steps" ref={group}>
      <button type="button" class="settings-btn seen-header-step" data-seen-step="previous" onClick={() => go(-1)}>
        {t("seen-previous-change")}
      </button>
      <button type="button" class="settings-btn seen-header-step" data-seen-step="next" onClick={() => go(1)}>
        {t("seen-next-change")}
      </button>
      <Show when={props.seen.position()}>
        {(at) => (
          <span class="seen-header-position" classList={{ wrapped: at().wrapped }} data-seen-position aria-live="polite">
            {at().wrapped ? t("seen-change-position-wrapped", { index: at().index, total: at().total }) : t("seen-change-position", { index: at().index, total: at().total })}
          </span>
        )}
      </Show>
    </span>
  );
}

/** "N changes since you last looked" + Previous / Next change + Mark page seen,
 *  only while N > 0. */
export function SeenHeader(props: { seen: PageSeen; pageName: string }): JSX.Element {
  const notes = () => [
    props.seen.identityKnown() ? "" : t("seen-note-identity-unknown"),
    props.seen.unattributed() ? t("seen-note-unattributed") : "",
    props.seen.reason() ?? "",
  ].filter(Boolean).join(" ");
  return (
    <Show when={props.seen.count() > 0}>
      <StickyNotice>
        <span class="seen-header-text" classList={{ "has-note": !!notes() }} title={notes() || undefined}>
          {t("seen-header-count", { count: props.seen.count() })}
        </span>
        <SeenSteps seen={props.seen} pageName={props.pageName} />
        <button type="button" class="settings-btn seen-header-mark" onClick={() => void markPageSeenNow(props.pageName, false)}>
          {t("seen-mark-page")}
        </button>
        <Show when={props.seen.unattributed()}>
          <div class="seen-header-muted" data-seen-unattributed>{t("seen-note-unattributed-line")}</div>
        </Show>
      </StickyNotice>
    </Show>
  );
}
