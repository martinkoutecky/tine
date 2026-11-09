// Width-only annotation entry points. Only annotated, mounted body rows own an
// observer; measuring a rail reads one rectangle per thread and writes offsets
// together in a coalesced frame. No document writes or additional source loads.
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, useContext, type JSX } from "solid-js";
import { Block, SurfaceContext } from "./Block";
import { MarginContext, type MarginApi } from "./marginContext";
import { CommentGroup } from "./CommentGroup";
import { PeekPopup } from "../render/PeekPopup";
import { commentTier } from "../margin";
import { commentGroups } from "../commentGroups";
import { node, formatForBlock, threadRootOf } from "../document";
import { editingId, editingSurface, endEdit } from "../editorController";
import { replyFocusRequest } from "../commentReplyFocus";
import { newResizeObserver, requestFrame, viewportOf } from "../windowRealm";
import { t } from "../i18n";
import { commentDraftState } from "../commentDraftState";
import { registerReplyDraft } from "../commentMargin";
import { threadResolved } from "../commentResolution";
import { useResolvedComments } from "./CommentResolutionScope";

export function MarginMarker(props: { parentId: string; comments: readonly string[] }): JSX.Element {
  return <Show when={props.comments.length > 0}><CommentEntry {...props} /></Show>;
}

function CommentEntry(props: { parentId: string; comments: readonly string[] }): JSX.Element {
  const showResolved = useResolvedComments();
  const place = useContext(MarginContext);
  const surface = useContext(SurfaceContext);
  const secondary = place?.surface !== surface && (surface.startsWith("ref:") || surface.startsWith("embed:"));
  const wide = () => !!place && place.thread === null && !secondary && place.api.active();
  let host!: HTMLSpanElement;
  let row: HTMLElement | null = null;
  const [width, setWidth] = createSignal(800);
  const phone = () => commentTier(width()) === "phone";
  const [opened, setOpened] = createSignal<{ id: string; anchor: HTMLElement }>();
  const [offsets, setOffsets] = createSignal<Record<string, number>>({});
  const groups = createMemo(() => {
    const available = new Set(props.comments);
    return commentGroups(props.parentId, formatForBlock(props.parentId), showResolved()).map(ids => ids.filter(id => available.has(id))).filter(ids => ids.length);
  });
  const openComments = createMemo(() => props.comments.filter(id => !threadResolved(id)));
  const resolvedComments = createMemo(() => props.comments.filter(threadResolved));
  const selectedId = () => props.comments.includes(opened()?.id ?? "") ? opened()?.id : groups()[0]?.[0];
  const index = () => Math.max(0, groups().findIndex(ids => ids.includes(selectedId() ?? "")));
  const openLabel = () => t("comment-margin-count", { count: openComments().length });
  const resolvedLabel = () => t("comment-resolved-count", { count: resolvedComments().length });
  const label = () => selectedId() && threadResolved(selectedId()!) ? resolvedLabel() : openLabel();
  const open = (id: string, anchor: HTMLElement) => setOpened({ id, anchor });
  const finishThreadEdit = () => {
    const id = editingId();
    if (id && (editingSurface() === null || editingSurface() === surface) && props.comments.includes(threadRootOf(id, formatForBlock(props.parentId)) ?? "")) endEdit("blur");
    const active = host.ownerDocument.activeElement as HTMLElement | null;
    if (active?.closest(".comment-thread-popup")) active.blur();
  };
  const close = () => { finishThreadEdit(); setOpened(undefined); host.querySelector<HTMLButtonElement>(".margin-marker,.comment-resolved-entry")?.focus({ preventScroll: true }); };
  const move = (step: number) => {
    const ids = groups()[index() + step];
    if (ids && opened()) { finishThreadEdit(); open(ids[0], opened()!.anchor); }
  };
  const api: MarginApi = place?.api ?? {
    active: () => false, register: () => () => {}, schedule: () => {}, activeComment: () => null, setActiveComment: () => {},
  };
  let frame: (() => void) | undefined;
  const measure = () => {
    frame = undefined;
    if (!row) return;
    const marks = [...row.querySelectorAll<HTMLElement>(".comment-quote-anchor[data-comment-id]")];
    const top = row.getBoundingClientRect().top;
    const next: Record<string, number> = {};
    const lanes = new Map<number, number>();
    for (const id of openComments()) {
      const peers = groups().find(ids => ids.includes(id)) ?? [id];
      const mark = marks.find(mark => mark.dataset.commentId === id) ?? marks.find(mark => peers.includes(mark.dataset.commentId!));
      const y = Math.max(0, Math.round((mark?.getBoundingClientRect().top ?? top) - top));
      // Same-line threads occupy adjacent slim columns, retaining alignment.
      const lane = lanes.get(y) ?? 0; lanes.set(y, lane + 1);
      next[id] = y; next[`${id}:lane`] = lane;
    }
    setOffsets(next);
    for (const mark of marks) {
      if (!wide()) { mark.tabIndex = 0; mark.setAttribute("role", "button"); mark.setAttribute("aria-label", t("comment-open-thread")); }
      else { mark.removeAttribute("tabindex"); mark.removeAttribute("role"); mark.removeAttribute("aria-label"); }
    }
    api.schedule();
  };
  const schedule = () => { if (!frame) frame = requestFrame(host, measure); };
  onCleanup(() => frame?.());
  onMount(() => {
    row = host.closest<HTMLElement>(".block-main");
    if (!row) return;
    if (place && !secondary) onCleanup(api.register(props.parentId, row));
    const pane = row.closest<HTMLElement>(".comment-surface, [data-sidebar-surface], .live-ref-group, .main-content, .right-sidebar") ?? row.parentElement!;
    const read = () => { setWidth(pane.clientWidth || viewportOf(pane).width); schedule(); };
    read();
    const observer = newResizeObserver(pane, read); observer?.observe(pane); observer?.observe(row);
    onCleanup(() => observer?.disconnect());
    const fonts = host.ownerDocument.fonts;
    fonts?.addEventListener?.("loadingdone", schedule);
    onCleanup(() => fonts?.removeEventListener?.("loadingdone", schedule));
    const Observer = (row.ownerDocument.defaultView as (Window & typeof globalThis) | null)?.MutationObserver;
    const watcher = Observer ? new Observer(schedule) : null;
    watcher?.observe(row.querySelector(".block-content-wrapper") ?? row, { childList: true, subtree: true, characterData: true });
    onCleanup(() => watcher?.disconnect());
    const markOf = (event: Event) => (event.target as Element | null)?.closest<HTMLElement>(".comment-quote-anchor[data-comment-id]");
    const down = (event: MouseEvent) => { if (!wide() && markOf(event)) event.stopPropagation(); };
    const click = (event: MouseEvent | KeyboardEvent) => {
      const mark = markOf(event);
      if (wide() || !mark || !props.comments.includes(mark.dataset.commentId!)) return;
      const keyboard = event instanceof host.ownerDocument.defaultView!.KeyboardEvent;
      if (keyboard && event.key !== "Enter" && event.key !== " ") return;
      if (!keyboard && host.ownerDocument.defaultView?.getSelection()?.toString()) return;
      event.preventDefault(); event.stopPropagation(); open(mark.dataset.commentId!, mark);
    };
    row.addEventListener("mousedown", down, true); row.addEventListener("click", click, true); row.addEventListener("keydown", click, true);
    onCleanup(() => { row?.removeEventListener("mousedown", down, true); row?.removeEventListener("click", click, true); row?.removeEventListener("keydown", click, true); });
  });
  createEffect(() => { void node(props.parentId)?.raw; props.comments; wide(); schedule(); });
  // Creation and Comment-on-existing requests must reveal the live thread even
  // when the old body editor has already unmounted. Preserve its surface key.
  createEffect(() => {
    if (wide()) return;
    const request = replyFocusRequest();
    const edit = editingId();
    const id = request && (request.surface === null || request.surface === surface) ? request.id
      : edit && (editingSurface() === null || editingSurface() === surface) ? threadRootOf(edit, formatForBlock(props.parentId)) : null;
    if (id && props.comments.includes(id) && !groups()[index()]?.includes(id)) open(id, host);
    else if (id && props.comments.includes(id) && !opened()) open(id, host);
  });
  let touch: { x: number; y: number } | undefined;
  const content = () => <div class="comment-popup-content"
    onTouchStart={event => { const first = event.touches[0]; touch = first && !(event.target as Element).closest("textarea,[contenteditable=true]") ? { x: first.clientX, y: first.clientY } : undefined; }}
    onTouchEnd={event => {
      const last = event.changedTouches[0];
      if (phone() && touch && last) {
        const dx = last.clientX - touch.x, dy = last.clientY - touch.y;
        if (Math.abs(dx) > 50 && Math.abs(dx) > Math.abs(dy) * 1.5) move(dx < 0 ? 1 : -1);
      }
      touch = undefined;
    }}>
    <div class="comment-popup-toolbar">
      <span>{t("comment-thread-position", { current: index() + 1, count: groups().length })}</span>
      <button type="button" class="comment-prev-thread" disabled={index() === 0} aria-label={t("comment-prev-thread")} onClick={() => move(-1)}>‹</button>
      <button type="button" class="comment-next-thread" disabled={index() === groups().length - 1} aria-label={t("comment-next-thread")} onClick={() => move(1)}>›</button>
      <button type="button" class="comment-popup-close" aria-label={t("comment-close-threads")} onClick={close}>×</button>
    </div>
    <SurfaceContext.Provider value={surface}>
      {/* Keep all of this block's composers mounted while switching threads. */}
      <For each={groups().map(ids => ids[0])}>{key => {
        const ids = () => groups().find(ids => ids[0] === key) ?? [];
        return <div hidden={!ids().includes(selectedId() ?? "")}>
          <CommentGroup ids={ids()} render={id => <MarginContext.Provider value={{ api, thread: { root: id, parent: props.parentId } }}><Block id={id} /></MarginContext.Provider>} />
        </div>;
      }}</For>
    </SurfaceContext.Provider>
  </div>;
  return <span class="comment-entry" ref={host}>
    <For each={props.comments}>{id => {
      const state = commentDraftState(id, surface);
      onCleanup(registerReplyDraft(state.draft, id));
      return null;
    }}</For>
    <Show when={!wide()}>
      <Show when={openComments().length > 0}><button type="button" class="margin-marker" title={openLabel()} aria-label={openLabel()}
        onMouseDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); open(openComments()[0], e.currentTarget); }}>{openComments().length}</button></Show>
      <Show when={resolvedComments().length > 0}><button type="button" class="comment-resolved-entry" title={resolvedLabel()} aria-label={resolvedLabel()}
        onMouseDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); open(resolvedComments()[0], e.currentTarget); }}>{resolvedLabel()}</button></Show>
      <Show when={!phone() && openComments().length > 0}><span class="comment-rail" aria-label={t("comment-margin-label")}>
        <For each={openComments()}>{id => <button type="button" class="comment-rail-marker" data-comment-marker={id}
          style={{ top: `${offsets()[id] ?? 0}px`, "inset-inline-end": `${(offsets()[`${id}:lane`] ?? 0) * 10}px` }} aria-label={t("comment-open-thread")}
          onMouseDown={e => e.stopPropagation()} onClick={e => { e.stopPropagation(); open(id, e.currentTarget); }} />}</For>
      </span></Show>
      <Show when={opened()}><PeekPopup anchor={() => opened()?.anchor} blocks={() => []}
        class={`comment-thread-popup${phone() ? " comment-bottom-sheet" : ""}`} label={label()} bottomSheet={phone()}
        onPointerEnter={() => {}} onPointerLeave={() => {}} onDismiss={close} content={content()} /></Show>
    </Show>
  </span>;
}
