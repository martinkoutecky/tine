import { Show, createEffect, createSignal, createUniqueId, onCleanup, type JSX } from "solid-js";
import { t } from "../i18n";
import { FloatingPortal } from "./FloatingPortal";
import { registerTransientLayer } from "../transientLayers";
import { windowOf } from "../windowRealm";

/** One native disclosure works with pointer, keyboard and touch. Actions live
 * here; the host owns all reads and the existing guarded writes. */
export function QueryActions(props: {
  editable: boolean;
  results: boolean;
  edit: (tab: "filter" | "display" | "text") => void;
  copy: () => void;
  search: () => void;
  export: () => void;
  fold?: (collapsed: boolean) => void;
  folded?: boolean;
}): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const [position, setPosition] = createSignal({ left: 0, top: 0 });
  const id = `query-actions-${createUniqueId()}`;
  let menu: HTMLDivElement | undefined;
  let details: HTMLDetailsElement | undefined;
  const close = () => { if (details) details.open = false; setOpen(false); };
  const run = (action: () => void) => { close(); action(); };
  createEffect(() => {
    if (!open()) return;
    const unregister = registerTransientLayer({ id, root: () => menu ?? null,
      trigger: () => details?.querySelector("summary") ?? null,
      dismiss: () => { close(); details?.querySelector("summary")?.focus(); return true; } });
    queueMicrotask(() => { if (open()) menu?.querySelector("button")?.focus(); });
    onCleanup(unregister);
  });
  const toggle = (element: HTMLDetailsElement) => {
    if (element.open) {
      const box = element.getBoundingClientRect();
      const win = windowOf(element);
      setPosition({ left: Math.max(4, Math.min(box.right - 180, win.innerWidth - 184)),
        top: Math.max(4, Math.min(box.bottom, win.innerHeight - 330)) });
    }
    setOpen(element.open);
  };
  return <details ref={details} class="qc-actions" onToggle={(event) => toggle(event.currentTarget)}
    onMouseDown={(event) => event.stopPropagation()} onClick={(event) => event.stopPropagation()}
    onKeyDown={(event) => {
      event.stopPropagation();
      if (event.key === "Escape") { close(); details?.querySelector("summary")?.focus(); }
    }}>
    <summary aria-label={t("query-calm-actions")} aria-controls={id}>⋯</summary>
    <Show when={open()}><FloatingPortal><div ref={menu} id={id} class="qc-menu"
      style={{ position: "fixed", left: `${position().left}px`, top: `${position().top}px`, right: "auto" }}
      onMouseDown={event => event.stopPropagation()} onClick={event => event.stopPropagation()}
      onKeyDown={event => { event.stopPropagation(); if (event.key === "Escape") { close(); details?.querySelector("summary")?.focus(); } }}>
      <Show when={props.editable}>
        <button type="button" onClick={() => run(() => props.edit("display"))}>{t("query-calm-view")}</button>
        <button type="button" onClick={() => run(() => props.edit("filter"))}>{t("query-calm-edit")}</button>
        <button type="button" onClick={() => run(() => props.edit("text"))}>{t("query-calm-text")}</button>
      </Show>
      <button type="button" disabled={!props.results} onClick={() => run(props.copy)}>{t("query-calm-copy")}</button>
      <Show when={props.fold}>
        <button type="button" disabled={!props.results} onClick={() => run(() => props.fold?.(!props.folded))}>{t(props.folded ? "query-calm-expand" : "query-calm-collapse")}</button>
      </Show>
      <button type="button" onClick={() => run(props.search)}>{t("query-calm-search")}</button>
      <button type="button" disabled={!props.results} onClick={() => run(props.export)}>{t("query-calm-export")}</button>
    </div></FloatingPortal></Show>
  </details>;
}
