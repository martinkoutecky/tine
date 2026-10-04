import { onCleanup, onMount, type JSX } from "solid-js";

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
 * Without a pane (exports/tests), the ordinary column scroll rule applies. */
export function TableWrap(props: { children: JSX.Element }): JSX.Element {
  let el!: HTMLDivElement;
  onMount(() => {
    const parent = el.parentElement;
    const pane = el.closest<HTMLElement>(".main-content, .rs-item-body, .right-sidebar-body");
    if (!parent || !pane || el.closest(".sheet-cell")) return;
    const surface = el.firstElementChild as HTMLElement;
    const column = el.closest<HTMLElement>(".main-content-inner") ?? parent;
    let frame = 0;
    const measure = () => {
      frame = 0;
      const p = parent.getBoundingClientRect(), m = pane.getBoundingClientRect(), c = column.getBoundingClientRect();
      const parentStyle = getComputedStyle(parent);
      const normalLeft = p.left + (parseFloat(parentStyle.paddingLeft) || 0);
      const normalWidth = parent.clientWidth - (parseFloat(parentStyle.paddingLeft) || 0) - (parseFloat(parentStyle.paddingRight) || 0);
      el.style.setProperty("--table-column-width", `${normalWidth}px`);
      const { width, shift } = tableBleedGeometry(normalLeft, normalWidth, surface.scrollWidth, m.left, pane.clientWidth, (c.left + c.right) / 2);
      el.style.setProperty("--table-bleed-width", `${width}px`);
      el.style.setProperty("--table-bleed-shift", `${shift}px`);
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    for (const node of [parent, pane, column, surface]) observer?.observe(node);
    window.addEventListener("resize", schedule);
    schedule();
    onCleanup(() => { observer?.disconnect(); cancelAnimationFrame(frame); window.removeEventListener("resize", schedule); });
  });
  return <div ref={el} class="md-table-wrap">{props.children}</div>;
}
