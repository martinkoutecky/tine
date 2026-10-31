/** Render sibling blocks with one shared shell window on every outline surface.
 * Small outlines and single-child chains render directly. Large lists estimate
 * geometry until first seen, then keep measured spacers while outside the viewport. Creation costs
 * O(descendant ids + raw text) per sibling list for geometry and registration;
 * nested lists repeat ancestor registration. Shells/cleanup follow mounted groups
 * and their rendered descendants (nearby, revealed, or editing groups; all for
 * printing or without IntersectionObserver). No document mutation or asynchronous
 * failure. IDs/content come from the document door; collapse follows the existing
 * Block surface context (or session folds for read-only sources), and
 * viewport/font come from DOM. Hosts register
 * descendant ids for model-driven reveal; callers provide only ids.
 */
import { For, Show, batch, createEffect, createSignal, onCleanup, onMount, untrack, useContext, type JSX } from "solid-js";
import { node } from "../document";
import { pageFoldSurfaces } from "./block/readOnlyFold";
import { editingId } from "../editorController";
import { clearOnBindingInvalidated } from "../binding";
import { listenForOutlinePrint, printingOutline, registerOutlineWindow } from "../outlineViewport";
import { Block, CollapseSurfaceContext, SurfaceContext } from "./Block";
import { elementFromPointIn, newIntersectionObserver, newResizeObserver, windowOf } from "../windowRealm";
import { createOutlineWindowGeometry, createOutlineWindowModel } from "./outlineWindowModel";
import { batchViewportChanges, preserveAboveViewport, cachedScrollContainer } from "../scrollContainer";

const GROUP = 24;
const WINDOW_AT = 80;
/** A list enters windowing above WINDOW_AT but, once windowed, stays so until
 * it shrinks by more than one group's weight: crossing 80 back and forth (an
 * outdent moves siblings out, an indent takes them back) would otherwise swap
 * windows for plain blocks and remount the whole list, collapsing every image
 * in it for a frame. */
const UNWINDOW_AT = WINDOW_AT - GROUP;
/** One shell window. Its identity (the `For` key) outlives edits to its
 * membership: a structural edit changes `ids` in place, so the blocks that
 * stayed keep their DOM, measured height and loaded images (GH #745). */
interface WindowGroup { readonly ids: () => readonly string[]; readonly set: (ids: readonly string[]) => void }
type OutlineEntry = string | { readonly groups: readonly WindowGroup[] };
// Bounded, graph-scoped geometry only: never retain components or document nodes.
const heights = new Map<string, { width: number; structure: string; version: number; height: number }>();
clearOnBindingInvalidated(() => heights.clear());

function newGroup(initial: readonly string[]): WindowGroup {
  const [ids, set] = createSignal<readonly string[]>(initial);
  return { ids, set };
}

const sameIds = (a: readonly string[], b: readonly string[]): boolean => a.length === b.length && a.every((id, i) => id === b[i]);

/** Partition `ids` into windows of about GROUP weight while keeping every
 * unchanged window (and its handle) as it was. Greedy re-packing from scratch
 * shifts every later boundary when one group overflows by a row; here each id
 * stays in its prior window, a new id joins its predecessor's, and only a
 * window that grew past 2*GROUP is split or that shrank is merged into its
 * neighbour. O(ids). Returns the handles in order and the ids each must hold. */
export function stablePartition(ids: readonly string[], sizes: readonly number[], prior: readonly WindowGroup[]):
  { handle: WindowGroup | null; ids: string[] }[] {
  const owner = new Map<string, number>();
  prior.forEach((g, gi) => untrack(g.ids).forEach(id => owner.set(id, gi)));
  const label = ids.map(id => owner.get(id) ?? -1);
  for (let i = 1; i < label.length; i++) if (label[i] < 0) label[i] = label[i - 1];
  for (let i = label.length - 2; i >= 0; i--) if (label[i] < 0) label[i] = label[i + 1];
  // A single block moved into another window's range stays with its neighbours.
  for (let i = 1; i + 1 < label.length; i++) if (label[i - 1] === label[i + 1] && label[i] !== label[i - 1]) label[i] = label[i - 1];
  type Run = { handle: WindowGroup | null; ids: string[]; weight: number };
  const claimed = new Set<number>();
  const runs: Run[] = [];
  ids.forEach((id, i) => {
    const last = runs[runs.length - 1];
    if (last && i > 0 && label[i] === label[i - 1]) { last.ids.push(id); last.weight += sizes[i]; return; }
    const handle = label[i] >= 0 && !claimed.has(label[i]) ? prior[label[i]] : null;
    if (label[i] >= 0) claimed.add(label[i]);
    runs.push({ handle, ids: [id], weight: sizes[i] });
  });
  const out: Run[] = [];
  const push = (run: Run) => {
    const previous = out[out.length - 1];
    if (previous && previous.weight + run.weight <= GROUP) { previous.ids.push(...run.ids); previous.weight += run.weight; }
    else out.push(run);
  };
  let cursor = 0;
  for (const run of runs) {
    const start = cursor; cursor += run.ids.length;
    if (run.weight <= 2 * GROUP || run.ids.length === 1) { push(run); continue; }
    let piece: Run = { handle: run.handle, ids: [], weight: 0 };
    run.ids.forEach((id, k) => {
      const w = sizes[start + k];
      if (piece.ids.length && piece.weight + w > GROUP) { out.push(piece); piece = { handle: null, ids: [], weight: 0 }; }
      piece.ids.push(id); piece.weight += w;
    });
    out.push(piece);
  }
  return out;
}

export function BlockList(props: { ids: readonly string[] }): JSX.Element {
  return <For {...blockListProps(() => props.ids)} />;
}

/** The same renderer as BlockList, as Solid For props for recursive callers.
 * Mount it directly on their existing For to preserve legal 128-level cleanup
 * depth. The getter stays reactive; this module owns grouping and rendering.
 * Pass these props intact; destructuring each takes a snapshot. Same costs
 * and lifetime as BlockList, no document mutation. */
export function blockListProps(idsOf: () => readonly string[]): { readonly each: readonly OutlineEntry[]; children: (entry: OutlineEntry) => JSX.Element } {
  onCleanup(listenForOutlinePrint());
  const collapseSurface = useContext(CollapseSurfaceContext);
  const [groups, setGroups] = createSignal<readonly WindowGroup[]>([]);
  // Keep the list owner across structural edits. For keys windows by their
  // handle, so an unchanged window keeps its mounted blocks and images, and a
  // touched one keeps every block that stayed in it.
  const windowed: readonly OutlineEntry[] = [{ get groups() { return groups(); } }];
  const plain = (ids: readonly string[]): readonly string[] => {
    if (untrack(groups).length) setGroups([]);
    return ids;
  };
  const entries = (): readonly OutlineEntry[] => {
    const ids = idsOf();
    if (ids.length <= 1) return plain(ids);
    // A shallow list can still own thousands of expanded descendants. Weight
    // groups by rendered subtree size, capped at the small-list threshold.
    const foldSurface = pageFoldSurfaces(collapseSurface);
    const size = (id: string): number => {
      const n = node(id);
      let count = 1;
      if (n && !(foldSurface(id, n.page)?.collapsed(id, n.collapsed) ?? n.collapsed)) for (const child of n.children) {
        count += size(child);
        if (count > WINDOW_AT) break;
      }
      return count;
    };
    const sizes = ids.map(size);
    const prior = untrack(groups);
    if (sizes.reduce((sum, value) => sum + value, 0) <= (prior.length ? UNWINDOW_AT : WINDOW_AT)) return plain(ids);
    // Typing/editor-state reads keep the same O(ids) comparison as before;
    // only a changed membership needs the partition below.
    let at = 0;
    const unchanged = prior.every(group => {
      const members = untrack(group.ids);
      let weight = 0;
      for (const id of members) { if (ids[at] !== id) return false; weight += sizes[at++]; }
      return members.length === 1 || weight <= 2 * GROUP;
    }) && at === ids.length;
    if (unchanged) return windowed;
    const next = stablePartition(ids, sizes, prior).map(part => {
      const handle = part.handle ?? newGroup(part.ids);
      if (!sameIds(untrack(handle.ids), part.ids)) handle.set(part.ids);
      return handle;
    });
    if (next.length !== prior.length || next.some((handle, i) => handle !== prior[i])) setGroups(next);
    return windowed;
  };
  // Use the existing For owner alone: legal deep outlines cannot gain another
  // reactive owner at every ancestor without overflowing Solid cleanup.
  return {
    get each() { return entries(); },
    children: (entry: OutlineEntry): JSX.Element => typeof entry === "string"
      ? <Block id={entry} /> : <WindowedList groups={entry.groups} />,
  };
}

function WindowedList(props: { groups: readonly WindowGroup[] }): JSX.Element {
  let container!: HTMLDivElement;
  const [width, setWidth] = createSignal(900);
  const [font, setFont] = createSignal("16px sans-serif");
  const callbacks = new WeakMap<Element, (entry: IntersectionObserverEntry) => void>();
  // Created after insertion from the list's own realm and vertical scroller.
  let observer: IntersectionObserver | null | undefined;
  onCleanup(() => observer?.disconnect());
  const observe = (host: HTMLElement, callback: (entry: IntersectionObserverEntry) => void) => {
    if (observer === undefined) {
      observer = newIntersectionObserver(host, (entries) => batchViewportChanges(() => {
        for (const entry of entries) callbacks.get(entry.target)?.(entry);
      }), { root: cachedScrollContainer(host), rootMargin: "600px 0px" });
    }
    callbacks.set(host, callback); observer?.observe(host);
    return () => { observer?.unobserve(host); callbacks.delete(host); };
  };
  onMount(() => {
    const update = () => batch(() => { setWidth(container.clientWidth || 900); setFont(getComputedStyle(container).font || "16px sans-serif"); });
    update();
    const resize = newResizeObserver(container, update);
    resize?.observe(container);
    onCleanup(() => resize?.disconnect());
  });
  return <div ref={container} class="outline-list"><For each={props.groups}>{(group, index) =>
    <BlockWindow ids={group.ids} first={index() === 0} width={width()} font={font()} observe={observe} />
  }</For></div>;
}

function BlockWindow(props: { ids: () => readonly string[]; first: boolean; width: number; font: string; observe: (host: HTMLElement, callback: (entry: IntersectionObserverEntry) => void) => () => void }): JSX.Element {
  const surface = useContext(SurfaceContext);
  const collapseSurface = useContext(CollapseSurfaceContext);
  const folds = () => {
    const foldSurface = pageFoldSurfaces(collapseSurface);
    return (id: string, page: string, stored: boolean) => foldSurface(id, page)?.collapsed(id, stored) ?? stored;
  };
  const cacheKey = () => `${surface}\0${props.width}\0${props.font}\0${props.ids()[0]}`;
  let host!: HTMLDivElement;
  const [near, setNear] = createSignal(props.first || typeof IntersectionObserver === "undefined");
  const [forced, setForced] = createSignal(false);
  const [height, setHeight] = createSignal(0);
  const model = createOutlineWindowModel(props.ids, folds);
  const geometry = createOutlineWindowGeometry(model, () => props.width, () => props.font);
  const active = () => {
    const id = editingId();
    return !!id && model().members.has(id);
  };
  const mounted = () => near() || forced() || active() || printingOutline();
  const remember = (measuredHeight?: number, measuredWidth?: number) => {
    if (!host?.isConnected || !untrack(mounted)) return;
    const h = measuredHeight ?? host.getBoundingClientRect().height;
    if (h <= 0) return;
    setHeight(h);
    const key = cacheKey();
    heights.delete(key);
    heights.set(key, { width: measuredWidth ?? host.clientWidth, structure: untrack(() => model().structure), version: untrack(() => geometry().version()), height: h });
    if (heights.size > 512) heights.delete(heights.keys().next().value!);
  };
  createEffect(() => {
    const current = model();
    if (!host) return;
    onCleanup(registerOutlineWindow(current.all, host, () => { if (!untrack(near)) setForced(true); }));
  });
  createEffect(() => {
    const current = model();
    const version = geometry().version();
    if (!host) return;
    const cached = heights.get(cacheKey());
    if (cached?.structure === current.structure && cached.version === version && cached.width === host.clientWidth) {
      setHeight(cached.height);
    } else {
      setHeight(0);
    }
  });
  onMount(() => {
    if (typeof IntersectionObserver === "undefined") return;
    const stopObserving = props.observe(host, (entry) => {
      preserveAboveViewport(host, () => {
        if (entry.isIntersecting) {
          setNear(true);
          setForced(false);
        } else if (!printingOutline()) {
          remember(entry.boundingClientRect?.height, entry.boundingClientRect?.width);
          setNear(false);
          // A reveal stays mounted until it has actually scrolled into view.
        }
      }, undefined, false, entry, 600);
    });
    // A fast scroll + immediate pointer action can precede the next IO callback.
    // Mount before the shared document-level selection/drop hit test executes.
    const warm = () => setNear(true);
    const press = (event: MouseEvent) => {
      if (untrack(mounted)) return;
      warm();
      const target = elementFromPointIn(host, event.clientX, event.clientY);
      if (target && host.contains(target)) {
        event.stopImmediatePropagation();
        target.dispatchEvent(new (windowOf(host) as Window & typeof globalThis).MouseEvent(event.type, event));
      }
    };
    host.addEventListener("mousemove", warm, true);
    host.addEventListener("mousedown", press, true);
    const resize = newResizeObserver(host, entries => {
      const size = entries[0]?.borderBoxSize?.[0];
      if (untrack(mounted)) remember(size?.blockSize, size?.inlineSize);
      else {
        const cached = heights.get(cacheKey());
        if (cached && cached.width !== host.clientWidth) { setHeight(0); }
      }
    });
    resize?.observe(host);
    onCleanup(() => {
      stopObserving(); resize?.disconnect();
      host.removeEventListener("mousemove", warm, true);
      host.removeEventListener("mousedown", press, true);
    });
  });
  return <div ref={host} class="outline-window" data-outline-window>
    <Show when={mounted()} fallback={
      <div aria-hidden="true" style={{ height: `${height() || geometry().estimate()}px` }} />
    }>
      <For each={props.ids()}>{(id) => <Block id={id} />}</For>
    </Show>
  </div>;
}
