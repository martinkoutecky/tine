import { onCleanup, onMount, type JSX } from "solid-js";
import { listen, newResizeObserver, requestFrame } from "../windowRealm";
import "./tableWrap.css";

/** Retain the long-cell marker for presentation consumers. Prose may also break
 * words, but only when whole words cannot fit the pane (md-table-tight);
 * numbers/unbroken tokens stay whole. GH #802: sizing follows available space,
 * not a character threshold. */
export const LONG_CELL_CHARS = 40;
export const longCellClass = (text: string): string | undefined =>
  [text.length > LONG_CELL_CHARS ? "md-cell-long" : "", /\s/.test(text.trim()) ? "md-cell-prose" : ""].filter(Boolean).join(" ") || undefined;

/** Given measured geometry, return the viewport width and leftward offset:
 * preserve fitting content; center wider content on the text column within
 * the pane with a 20px gutter. O(1), no DOM/domain reads or writes. */
export function tableBleedGeometry(normalLeft: number, normalWidth: number, naturalWidth: number, paneLeft: number, paneWidth: number, center: number): { width: number; shift: number } {
  const available = Math.max(0, paneWidth - 40);
  const width = Math.min(Math.max(normalWidth, naturalWidth), available);
  const left = naturalWidth <= normalWidth && normalWidth <= available ? normalLeft
    : Math.max(paneLeft + 20, Math.min(center - width / 2, paneLeft + paneWidth - 20 - width));
  return { width, shift: normalLeft - left };
}

/** Horizontal viewport for rendered tables. It observes only its content,
 * column and pane; cleanup releases observers and the pending animation frame.
 * Without a pane (exports/tests), the ordinary column scroll rule applies.
 * A table inside a query box or a sheet cell never bleeds: the box is its
 * viewport (the wrap is 100% of it and scrolls inside it, tableWrap.css). */
export function TableWrap(props: { children: JSX.Element }): JSX.Element {
  let el!: HTMLDivElement;
  onMount(() => {
    // A table can mount before its block is attached to the page (a Linked
    // References copy in the sidebar, GH #802): no pane is found yet, so set up
    // when the wrap is first laid out, never once-and-give-up.
    if (el.isConnected) { onCleanup(setup()); return; }
    let dispose: (() => void) | undefined;
    const waiter = newResizeObserver(el, () => {
      if (!el.isConnected || dispose) return;
      waiter?.disconnect();
      dispose = setup();
    });
    waiter?.observe(el);
    onCleanup(() => { waiter?.disconnect(); dispose?.(); });
  });
  const setup = (): (() => void) => {
    const parent = el.parentElement;
    const pane = el.closest<HTMLElement>(".main-content, .rs-item-body, .right-sidebar-body");
    if (!parent || !pane || el.closest(".sheet-cell") || el.closest(".query-block, .query-results-region")) {
      el.classList.remove("md-table-tight");
      return () => {};
    }
    const surface = el.firstElementChild as HTMLElement;
    const column = el.closest<HTMLElement>(".main-content-inner") ?? parent;
    let frame: (() => void) | undefined;
    const measure = () => {
      const p = parent.getBoundingClientRect(), m = pane.getBoundingClientRect(), c = column.getBoundingClientRect();
      const parentStyle = getComputedStyle(parent);
      const normalLeft = p.left + (parseFloat(parentStyle.paddingLeft) || 0);
      const normalWidth = parent.clientWidth - (parseFloat(parentStyle.paddingLeft) || 0) - (parseFloat(parentStyle.paddingRight) || 0);
      el.style.setProperty("--table-column-width", `${normalWidth}px`);
      // Measure intrinsic content before constraining the table to its viewport.
      // Otherwise a previously wrapped table reports its constrained width and
      // never grows back into newly available pane space after a resize.
      const maxWidth = surface.style.maxWidth;
      surface.style.maxWidth = "none";
      const naturalWidth = surface.scrollWidth;
      surface.style.maxWidth = maxWidth;
      const { width, shift } = tableBleedGeometry(normalLeft, normalWidth, naturalWidth, m.left, pane.clientWidth, (c.left + c.right) / 2);
      el.style.setProperty("--table-bleed-width", `${width}px`);
      // CSS anchors a block at inline-start. Geometry above is physical: an
      // RTL viewport starts at normalRight, so its right margin must also
      // account for the new width, not just negate the leftward shift.
      const inlineShift = parentStyle.direction === "rtl" ? width - normalWidth - shift : shift;
      el.style.setProperty("--table-bleed-shift", `${inlineShift}px`);
      // GH #802: split words only when whole words cannot fit the viewport
      // (tableWrap.css, md-table-tight). Decide with the class off each time,
      // so a wider pane returns to whole words.
      el.classList.remove("md-table-tight");
      if (surface.scrollWidth > el.clientWidth + 1) el.classList.add("md-table-tight");
    };
    const schedule = () => { if (!frame) frame = requestFrame(el, () => { frame = undefined; measure(); }); };
    const observer = newResizeObserver(el, schedule);
    for (const node of [parent, pane, column, surface]) observer?.observe(node);
    const stopResize = listen(el, "resize", schedule);
    schedule();
    return () => { observer?.disconnect(); frame?.(); stopResize(); };
  };
  // Reading copies can be inserted between layout frames (Linked References
  // in the sidebar). Fit prose immediately, before the observer/first frame
  // supplies pane geometry; measure() restores whole words whenever they fit.
  return <div ref={el} class="md-table-wrap md-table-tight">{props.children}</div>;
}
