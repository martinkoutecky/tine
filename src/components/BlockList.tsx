/** Render sibling blocks with one shared shell window on every outline surface.
 * Small outlines and single-child chains render directly. Large lists estimate
 * geometry until first seen, then keep measured spacers while outside the viewport. Creation costs
 * O(descendant ids + raw text) per sibling list for geometry and registration;
 * nested lists repeat ancestor registration. Shells/cleanup follow mounted groups
 * and their rendered descendants (nearby, revealed, or editing groups; all for
 * printing or without IntersectionObserver). No document mutation or asynchronous
 * failure. IDs/content come from the document door; collapse follows the existing
 * Block surface context, and viewport/font come from DOM. Hosts register
 * descendant ids for model-driven reveal; callers provide only ids.
 */
import { For, Show, batch, createEffect, createMemo, createSignal, onCleanup, onMount, untrack, useContext, type JSX } from "solid-js";
import { node } from "../document";
import { editingId } from "../editorController";
import { clearOnBindingInvalidated } from "../binding";
import { listenForOutlinePrint, printingOutline, registerOutlineWindow } from "../outlineViewport";
import { Block, CollapseSurfaceContext, SurfaceContext } from "./Block";

const GROUP = 24;
const WINDOW_AT = 80;
type OutlineEntry = string | { readonly groups: readonly (readonly string[])[] };
// Bounded, graph-scoped geometry only: never retain components or document nodes.
const heights = new Map<string, { width: number; signature: string; height: number }>();
clearOnBindingInvalidated(() => { heights.clear(); textWidths.clear(); });

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
  let previous: readonly OutlineEntry[] = [];
  const entries = (): readonly OutlineEntry[] => {
    const ids = idsOf();
    if (ids.length <= 1) return previous = ids;
    // A shallow list can still own thousands of expanded descendants. Weight
    // groups by rendered subtree size, capped at the small-list threshold.
    const size = (id: string): number => {
      const n = node(id);
      let count = 1;
      if (n && !(collapseSurface?.collapsed(id, n.collapsed) ?? n.collapsed)) for (const child of n.children) {
        count += size(child);
        if (count > WINDOW_AT) break;
      }
      return count;
    };
    const sizes = ids.map(size);
    if (sizes.reduce((sum, value) => sum + value, 0) <= WINDOW_AT) return previous = ids;
    const out: string[][] = [];
    let group: string[] = [], weight = 0;
    ids.forEach((id, i) => {
      if (group.length && weight + sizes[i] > GROUP) { out.push(group); group = []; weight = 0; }
      group.push(id); weight += sizes[i];
    });
    if (group.length) out.push(group);
    const next: readonly OutlineEntry[] = [{ groups: out }];
    // Retain group identities when a reactive root getter reads editor state.
    const same = previous.length === next.length && previous.every((entry, i) => {
      const other = next[i];
      if (typeof entry === "string") return entry === other;
      return typeof other !== "string" && entry.groups.length === other.groups.length
        && entry.groups.every((ids, g) => ids.length === other.groups[g].length
          && ids.every((id, j) => id === other.groups[g][j]));
    });
    return same ? previous : previous = next;
  };
  // Use the existing For owner alone: legal deep outlines cannot gain another
  // reactive owner at every ancestor without overflowing Solid cleanup.
  return {
    get each() { return entries(); },
    children: (entry: OutlineEntry): JSX.Element => typeof entry === "string"
      ? <Block id={entry} /> : <WindowedList groups={entry.groups} />,
  };
}

function WindowedList(props: { groups: readonly (readonly string[])[] }): JSX.Element {
  let container!: HTMLDivElement;
  const [width, setWidth] = createSignal(900);
  const [font, setFont] = createSignal("16px sans-serif");
  const callbacks = new WeakMap<Element, (near: boolean) => void>();
  const observer = typeof IntersectionObserver === "undefined" ? null : new IntersectionObserver((entries) => {
    for (const entry of entries) callbacks.get(entry.target)?.(entry.isIntersecting);
  }, { rootMargin: "600px 0px" });
  onCleanup(() => observer?.disconnect());
  const observe = (host: HTMLElement, callback: (near: boolean) => void) => {
    callbacks.set(host, callback); observer?.observe(host);
    return () => { observer?.unobserve(host); callbacks.delete(host); };
  };
  onMount(() => {
    const update = () => batch(() => { setWidth(container.clientWidth || 900); setFont(getComputedStyle(container).font || "16px sans-serif"); });
    update();
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    resize?.observe(container);
    onCleanup(() => resize?.disconnect());
  });
  return <div ref={container} class="outline-list"><For each={props.groups}>{(ids, index) =>
    <BlockWindow ids={ids} first={index() === 0} width={width()} font={font()} observe={observe} />
  }</For></div>;
}

function BlockWindow(props: { ids: readonly string[]; first: boolean; width: number; font: string; observe: (host: HTMLElement, callback: (near: boolean) => void) => () => void }): JSX.Element {
  const surface = useContext(SurfaceContext);
  const collapseSurface = useContext(CollapseSurfaceContext);
  const collapsed = (id: string, stored: boolean) => collapseSurface?.collapsed(id, stored) ?? stored;
  const cacheKey = () => `${surface}\0${props.width}\0${props.font}\0${props.ids[0]}`;
  let host!: HTMLDivElement;
  const [near, setNear] = createSignal(props.first || typeof IntersectionObserver === "undefined");
  const [forced, setForced] = createSignal(false);
  const [height, setHeight] = createSignal(0);
  const model = createMemo(() => {
    const all: string[] = [];
    const signature: string[] = [];
    const visit = (id: string) => {
      const n = node(id);
      if (!n) return;
      all.push(id);
      signature.push(id, n.raw, String(collapsed(id, n.collapsed)));
      n.children.forEach(visit);
    };
    props.ids.forEach(visit);
    return { all, members: new Set(all), signature: signature.join("\0") };
  });
  const active = () => {
    const id = editingId();
    return !!id && model().members.has(id);
  };
  const mounted = () => near() || forced() || active() || printingOutline();
  const remember = () => {
    if (!host?.isConnected || !untrack(mounted)) return;
    const h = host.getBoundingClientRect().height;
    if (h <= 0) return;
    setHeight(h);
    const key = cacheKey();
    heights.delete(key);
    heights.set(key, { width: host.clientWidth, signature: untrack(() => model().signature), height: h });
    if (heights.size > 512) heights.delete(heights.keys().next().value!);
  };
  createEffect(() => {
    const current = model();
    props.width; props.font;
    if (!host) return;
    onCleanup(registerOutlineWindow(current.all, host, () => { if (!untrack(near)) setForced(true); }));
    const cached = heights.get(cacheKey());
    if (cached?.signature === current.signature && cached.width === host.clientWidth) {
      setHeight(cached.height);
    } else {
      setHeight(0);
    }
  });
  onMount(() => {
    if (typeof IntersectionObserver === "undefined") return;
    const stopObserving = props.observe(host, (isNear) => {
      if (isNear) {
        setNear(true);
        setForced(false);
      } else if (!printingOutline()) {
        remember();
        setNear(false);
        // A reveal stays mounted until it has actually scrolled into view.
      }
    });
    // A fast scroll + immediate pointer action can precede the next IO callback.
    // Mount before the shared document-level selection/drop hit test executes.
    const warm = () => setNear(true);
    const press = (event: MouseEvent) => {
      if (untrack(mounted)) return;
      warm();
      const target = document.elementFromPoint(event.clientX, event.clientY);
      if (target && host.contains(target)) {
        event.stopImmediatePropagation();
        target.dispatchEvent(new MouseEvent(event.type, event));
      }
    };
    host.addEventListener("mousemove", warm, true);
    host.addEventListener("mousedown", press, true);
    const resize = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => {
      if (untrack(mounted)) remember();
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
      <div aria-hidden="true" style={{ height: `${height() || estimateHeight(props.ids, props.width, props.font, collapsed)}px` }} />
    }>
      <For each={props.ids}>{(id) => <Block id={id} />}</For>
    </Show>
  </div>;
}

// Text layout estimate only; never infer Logseq structure. Text samples are
// bounded and shared, so a page of long paragraphs needs neither offscreen DOM
// nor repeated font shaping. Actual heights replace estimates on first visit.
const textWidths = new Map<string, number>();
let measuringFont = "";
let canvas: CanvasRenderingContext2D | null | undefined;
function estimateHeight(ids: readonly string[], width: number, font: string, collapsed: (id: string, stored: boolean) => boolean): number {
  if (canvas === undefined) canvas = typeof CanvasRenderingContext2D === "undefined" ? null : document.createElement("canvas").getContext("2d");
  if (font !== measuringFont) {
    measuringFont = font; textWidths.clear();
    if (canvas) canvas.font = font;
  }
  const available = Math.max(80, width - 42);
  let height = 0;
  for (const id of ids) {
    const n = node(id);
    if (!n) continue;
    let lines = 0;
    for (const line of n.raw.split("\n")) {
      // Shape a bounded sample rather than every word of a long paragraph.
      // Estimates may change once on first view; revisits use exact geometry.
      const sample = line.slice(0, 160);
      let size = textWidths.get(sample);
      if (size === undefined) {
        size = canvas?.measureText(sample).width ?? sample.length * 8;
        if (textWidths.size >= 4096) textWidths.clear();
        textWidths.set(sample, size);
      }
      const length = sample.length ? size * line.length / sample.length : 0;
      lines += Math.max(1, Math.ceil(length / available));
    }
    height += Math.max(1, lines) * 26 + 2;
    if (!collapsed(id, n.collapsed)) height += estimateHeight(n.children, width - 20, font, collapsed);
  }
  return height;
}
