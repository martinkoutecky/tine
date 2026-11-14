import { For, Show, createEffect, createSignal, onCleanup, onMount, untrack, type JSX } from "solid-js";
import { backend } from "../backend";
import { admitRead, feedNames, mainPages, pageByName, retireFeedPages, ensurePageLoaded, toLoadablePage, emptyPage, reportPageLoadRefusal, isDirty, isSaving, isConflicted, topSelected, type FeedPage } from "../document";
import { editingId } from "../editorController";
import { graphOwner } from "../owned";
import { newIntersectionObserver, newResizeObserver, requestFrame, useOwnerWindow } from "../windowRealm";
import { captureReadingViewport } from "./readingViewportAnchor";
import { t } from "../i18n";
import { reportUiFailure } from "../uiFailure";
import { clearOnBindingInvalidated } from "../binding";
import "../styles/journalFeed.css";

type Slot = { name: string; el: HTMLElement; near: boolean; height: number; order: number; measure: () => void };
// All mounted surfaces consent to retirement: a split/window can be reading
// another part of the same shared feed. Slots retain geometry, never DTOs/text.
const slots = new Map<string, Set<Slot>>();
const eligibleSlots = new Map<string, Set<Slot>>();
let slotOrder = 0;
// Geometry follows feed membership across pane disposal/reopening. A new
// Journals surface must not start all already-retired days at zero height.
const heights = new Map<string, number>();
clearOnBindingInvalidated(() => { slots.clear(); eligibleSlots.clear(); heights.clear(); });

function reconsider(name: string): void {
  const group = slots.get(name);
  if (group?.size && pageByName(name) && [...group].every(slot => !slot.near && slot.height > 0)) eligibleSlots.set(name, group);
  else eligibleSlots.delete(name);
}

function forgetSlot(slot: Slot): void {
  const group = slots.get(slot.name);
  group?.delete(slot);
  if (!group?.size) slots.delete(slot.name);
  reconsider(slot.name);
}

export function JournalFeedDays(props: { children: (page: FeedPage) => JSX.Element }): JSX.Element {
  const win = useOwnerWindow();
  const local = new Map<Element, { slot: Slot; update: (near: boolean) => void }>();
  let observer: IntersectionObserver | null | undefined;
  let cancelFrame: (() => void) | undefined;
  const sweep = () => {
    cancelFrame = undefined;
    // Keep the former registration order, including when the first surface for
    // a name disappears. Only loaded, measured, far-away groups participate.
    const candidates = [...eligibleSlots].sort((a, b) => a[1].values().next().value!.order - b[1].values().next().value!.order);
    const eligible = candidates.filter(([, group]) => [...group].every(slot => {
      const selection = slot.el.ownerDocument.getSelection();
      if (selection && !selection.isCollapsed) for (let i = 0; i < selection.rangeCount; i++) {
        if (selection.getRangeAt(i).intersectsNode(slot.el)) return false;
      }
      return true;
    }));
    for (const [, group] of eligible) for (const slot of group) slot.measure();
    // Exact measured boxes are already reserved before document publication.
    untrack(() => retireFeedPages(eligible.map(([name]) => name)));
  };
  const schedule = (name?: string) => {
    if (name !== undefined) untrack(() => reconsider(name));
    if (!cancelFrame) cancelFrame = requestFrame(win, sweep);
  };
  const observe = (slot: Slot, update: (near: boolean) => void) => {
    if (observer === undefined) observer = newIntersectionObserver(win, entries => {
      for (const entry of entries) {
        const held = local.get(entry.target);
        if (!held) continue;
        held.slot.near = entry.isIntersecting;
        held.slot.measure();
        held.update(entry.isIntersecting);
        schedule(held.slot.name);
      }
      schedule();
    }, { root: slot.el.closest(".main-content"), rootMargin: "1200px 0px" });
    local.set(slot.el, { slot, update });
    let group = slots.get(slot.name);
    if (!group) slots.set(slot.name, group = new Set());
    group.add(slot);
    observer?.observe(slot.el);
    // No layout observer (tests/SSR): render eagerly, never guess geometry.
    if (!observer) { slot.near = true; update(true); }
    createEffect(() => {
      pageByName(slot.name);
      untrack(() => schedule(slot.name));
    });
    return () => { observer?.unobserve(slot.el); local.delete(slot.el); forgetSlot(slot); };
  };
  onCleanup(() => {
    cancelFrame?.(); observer?.disconnect();
    for (const { slot } of local.values()) forgetSlot(slot);
  });
  createEffect(() => {
    const names = new Set(feedNames());
    for (const name of heights.keys()) if (!names.has(name)) heights.delete(name);
  });
  // Reconsider formerly protected pages after their edit/save/selection ends.
  createEffect(() => {
    editingId(); topSelected();
    for (const page of mainPages()) { isDirty(page.name); isSaving(page.name); isConflicted(page.name); }
    schedule();
  });
  return <For each={feedNames()}>{name => <JournalDay name={name} observe={observe} schedule={schedule}>
    {props.children}
  </JournalDay>}</For>;
}

function JournalDay(props: { name: string; children: (page: FeedPage) => JSX.Element;
  observe: (slot: Slot, update: (near: boolean) => void) => () => void; schedule: (name?: string) => void }): JSX.Element {
  let el!: HTMLDivElement;
  let live = true;
  let inFlight = false;
  let path: string | undefined;
  const [near, setNear] = createSignal(false);
  const [height, setHeight] = createSignal(heights.get(props.name) ?? 0);
  const [reserve, setReserve] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  let resize: ResizeObserver | null | undefined;
  const page = () => pageByName(props.name);
  const releaseReserveIfFilled = () => {
    const body = el.firstElementChild;
    if (!body || !page()) return;
    const style = el.ownerDocument.defaultView!.getComputedStyle(body);
    const natural = body.getBoundingClientRect().height + (parseFloat(style.marginTop) || 0) + (parseFloat(style.marginBottom) || 0);
    if (natural >= height() - 0.5) setReserve(false);
  };
  const reload = async () => {
    if (inFlight || page()) return;
    inFlight = true; setFailed(false);
    try {
      await admitRead(graphOwner(() => live), () => path ? backend().getPageByPath(path) : backend().getPage(props.name, "journal"), dto => [props.name, ...(dto ? [dto.name] : [])], dto => {
        const anchor = captureReadingViewport(el.parentElement!);
        const refusal = ensurePageLoaded(toLoadablePage(dto ?? emptyPage(props.name, "journal"), props.name));
        anchor?.restore();
        requestFrame(el, () => { anchor?.restore(); anchor?.release(); });
        if (refusal) { reportPageLoadRefusal(refusal); setFailed(true); }
      });
    } catch (error) {
      if (live) { setFailed(true); reportUiFailure("page-refresh", error); }
    } finally { inFlight = false; }
  };
  createEffect(() => {
    const loaded = page();
    if (loaded) path = loaded.id;
    else setReserve(true);
    if (near() && !loaded && !failed()) void untrack(reload);
  });
  // Today's async sections can briefly be shorter during remount. Keep the
  // measured reserve until the body fills it; observe the body, since the
  // reserved wrapper itself does not shrink. User edits/folds may shrink normally.
  createEffect(() => {
    resize?.disconnect(); resize = null;
    if (!page()) return;
    const cancel = requestFrame(el, () => {
      resize?.disconnect();
      if (!live || !el.firstElementChild) return;
      resize = newResizeObserver(el, releaseReserveIfFilled);
      resize?.observe(el.firstElementChild);
      releaseReserveIfFilled();
    });
    onCleanup(cancel);
  });
  onCleanup(() => resize?.disconnect());
  onMount(() => {
    const slot: Slot = { name: props.name, el, near: true, height: height(), order: ++slotOrder, measure: () => {
      if (!page()) return;
      const measured = el.getBoundingClientRect().height;
      if (measured > 0) { slot.height = measured; heights.set(props.name, measured); setHeight(measured); }
    } };
    onCleanup(props.observe(slot, setNear));
    slot.measure(); props.schedule(props.name);
    onCleanup(() => { live = false; });
  });
  return <div ref={el} class="journal-feed-day" data-feed-day={props.name} data-feed-retired={!page() || undefined}
    onPointerDown={() => setReserve(false)} onKeyDown={() => setReserve(false)}
    style={{ display: "flow-root", "min-height": (!page() || reserve()) && height() ? `${height()}px` : undefined }}>
    <Show when={page()} keyed fallback={<Show when={failed()}>
      <button type="button" onClick={() => { setFailed(false); void reload(); }}>{t("resource-failure-retry")}</button>
    </Show>}>{loaded => props.children(loaded)}</Show>
  </div>;
}
