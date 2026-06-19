import { For, Show, createSignal, onMount, type JSX } from "solid-js";
import { pagePropsPanel, closePageProps } from "../ui";
import { readPageProperty, setPageProperty } from "../store";
import { PAGE_PROP_SPECS, type PagePropSpec } from "../editor/properties";

// Page-properties panel: labelled fields for the page-level properties that are
// otherwise only reachable as raw `key:: value` lines (alias, tags, public, …).
// Each field reads the current pre-block value and writes back through the store
// (undo-safe, persisted via the normal save path). Opened from the title gear or
// the "/Page properties" command.
export function PageProps(): JSX.Element {
  return (
    <Show when={pagePropsPanel()}>
      {(p) => <Panel name={p().name} x={p().x} y={p().y} />}
    </Show>
  );
}

function Panel(props: { name: string; x: number; y: number }): JSX.Element {
  const w = typeof window !== "undefined" ? window.innerWidth : 1280;
  const h = typeof window !== "undefined" ? window.innerHeight : 800;
  const left = Math.max(8, Math.min(props.x, w - 332));
  // Anchor at the click, then once mounted lift the panel up by its measured
  // height so its full content stays on-screen — no scrollbar for normal content.
  const [top, setTop] = createSignal(Math.max(8, Math.min(props.y, h - 380)));
  let el: HTMLDivElement | undefined;
  onMount(() => setTop(Math.max(8, Math.min(props.y, h - (el?.offsetHeight ?? 380) - 8))));
  return (
    <div
      class="pp-overlay"
      onClick={closePageProps}
      onContextMenu={(e) => {
        e.preventDefault();
        closePageProps();
      }}
    >
      <div ref={el} class="page-props-panel" style={{ left: `${left}px`, top: `${top()}px` }} onClick={(e) => e.stopPropagation()}>
        <div class="pp-head">
          Page properties <span class="pp-page">{props.name}</span>
        </div>
        <For each={PAGE_PROP_SPECS}>{(spec) => <Field name={props.name} spec={spec} />}</For>
        <div class="pp-foot">
          <button class="pp-done" onClick={closePageProps}>Done</button>
        </div>
      </div>
    </div>
  );
}

function Field(props: { name: string; spec: PagePropSpec }): JSX.Element {
  const initial = readPageProperty(props.name, props.spec.key) ?? "";

  if (props.spec.kind === "bool") {
    const [on, setOn] = createSignal(initial.toLowerCase() === "true");
    return (
      <label class="pp-field pp-bool">
        <input
          type="checkbox"
          checked={on()}
          onChange={(e) => {
            setOn(e.currentTarget.checked);
            setPageProperty(props.name, props.spec.key, e.currentTarget.checked ? "true" : null);
          }}
        />
        <span class="pp-text">
          <span class="pp-label">{props.spec.label}</span>
          <span class="pp-hint">{props.spec.hint}</span>
        </span>
      </label>
    );
  }

  const [v, setV] = createSignal(initial);
  // Only write on an actual local edit. Otherwise blurring/closing the panel
  // re-commits the value read when it opened — clobbering a concurrent external
  // edit (OG/Syncthing) that the file-watcher reloaded while the panel was open.
  const commit = () => {
    if (v() === initial) return;
    setPageProperty(props.name, props.spec.key, v().trim() || null);
  };
  return (
    <div class="pp-field">
      <label class="pp-label">{props.spec.label}</label>
      <input
        class="pp-input"
        value={v()}
        placeholder={props.spec.kind === "list" ? "comma, separated" : ""}
        onInput={(e) => setV(e.currentTarget.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") {
            commit();
            closePageProps();
          } else if (e.key === "Escape") {
            closePageProps();
          }
        }}
        onBlur={commit}
      />
      <div class="pp-hint">{props.spec.hint}</div>
    </div>
  );
}
