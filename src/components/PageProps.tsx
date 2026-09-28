import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { pagePropsPanel, closePageProps, type PropsPanelScope } from "../ui";
import {
  blockPageReadOnly, blockProperty, formatForBlock, node, pageByName,
  readPageProperties, readPageProperty, setBlockProperty, setPageProperty,
} from "../document";
import { PAGE_PROP_SPECS, isEditablePropertyKey, isSheetCellHidden, type PagePropSpec } from "../editor/properties";
import { facetsOf } from "../render/facets";
import { dismissTopTransient, registerTransientLayer } from "../transientLayers";
import { stillBound, type Binding } from "../binding";
import "../styles/props-panel.css";

// Properties panel: labelled fields for the properties of a page's pre-block or
// of one block. Every field reads the current value and writes back through the
// document (undo-safe, persisted via the normal save path). Opened from the page
// title gear, the "/Page properties" command, the page menu, or a block's menu.
// GH #164 (master eae864acb): ANY key the file has, plus an add-row; the five
// page presets stay first; an undeclared key labels itself and edits as text.
export function PageProps(): JSX.Element {
  return (
    <Show when={pagePropsPanel()} keyed>
      {(p) => <Panel scope={p.scope} x={p.x} y={p.y} binding={p.binding} />}
    </Show>
  );
}

// Machine-managed keys (`id`, `collapsed`, `logseq.order-list-type`, `tine.*`)
// keep their own surfaces. Same question as the sheet-cell editor asks, so it
// delegates rather than minting a second hidden set.
const machineManaged = (key: string) => isSheetCellHidden(key.toLowerCase());

function existingProperties(scope: PropsPanelScope): [string, string][] {
  if (scope.kind === "page") return readPageProperties(scope.name);
  const n = node(scope.id);
  return n ? facetsOf(n.raw, formatForBlock(scope.id)).properties : [];
}

const readOne = (scope: PropsPanelScope, key: string) =>
  scope.kind === "page" ? readPageProperty(scope.name, key) : blockProperty(scope.id, key);

function writeOne(scope: PropsPanelScope, binding: Binding, key: string, value: string | null): void {
  if (!stillBound(binding)) return;
  if (scope.kind === "page") setPageProperty(scope.name, key, value);
  else setBlockProperty(scope.id, key, value);
}

/** Refuses only when the subject is LOADED and positively read-only; an
 *  unloaded subject is unknown, and the writers are already no-ops for it. */
function scopeWritable(scope: PropsPanelScope): boolean {
  if (scope.kind === "block") return node(scope.id) ? !blockPageReadOnly(scope.id) : true;
  const page = pageByName(scope.name);
  return !page || (!page.readOnly && !page.guide);
}

function scopeLabel(scope: PropsPanelScope): { title: string; subject: string } {
  if (scope.kind === "page") return { title: "Page properties", subject: scope.name };
  const first = (node(scope.id)?.raw ?? "").split("\n")[0]?.trim() ?? "";
  return { title: "Block properties", subject: first.length > 48 ? `${first.slice(0, 48)}…` : first };
}

/** Presets (page scope only, shown even when absent) then every other property
 *  the scope has. Presets stay FIRST so the first `.pp-input` is a preset. */
function rowsFor(scope: PropsPanelScope): PagePropSpec[] {
  const rows = scope.kind === "page" ? [...PAGE_PROP_SPECS] : [];
  const seen = new Set(rows.map((spec) => spec.key.toLowerCase()));
  for (const [key] of existingProperties(scope)) {
    const lower = key.toLowerCase();
    if (seen.has(lower) || machineManaged(lower)) continue;
    seen.add(lower);
    rows.push({ key, label: key, hint: "", kind: "text" });
  }
  return rows;
}

const sameKeys = (a: PagePropSpec[], b: PagePropSpec[]) =>
  a.length === b.length && a.every((row, i) => row.key === b[i].key);

function Panel(props: { scope: PropsPanelScope; x: number; y: number; binding: Binding }): JSX.Element {
  const w = typeof window !== "undefined" ? window.innerWidth : 1280;
  const h = typeof window !== "undefined" ? window.innerHeight : 800;
  const left = Math.max(8, Math.min(props.x, w - 332));
  // Anchor at the click, then once mounted lift the panel up by its measured
  // height so its full content stays on-screen — no scrollbar for normal content.
  const [top, setTop] = createSignal(Math.max(8, Math.min(props.y, h - 380)));
  let el: HTMLDivElement | undefined;
  createEffect(() => {
    const unregister = registerTransientLayer({ id: "page-properties", root: () => el ?? null, dismiss: () => { closePageProps(); return true; } });
    onCleanup(unregister);
  });
  onMount(() => setTop(Math.max(8, Math.min(props.y, h - (el?.offsetHeight ?? 380) - 8))));
  // Keyed by the KEY SET: an unrelated store change (reload, save) must not
  // remount the fields and discard what the user is halfway through typing.
  const rows = createMemo(() => rowsFor(props.scope), undefined, { equals: sameKeys });
  const writable = createMemo(() => scopeWritable(props.scope));
  const heading = createMemo(() => scopeLabel(props.scope));
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
          {heading().title} <span class="pp-page">{heading().subject}</span>
        </div>
        <Show
          when={writable()}
          fallback={<div class="pp-hint">This {props.scope.kind} is read-only, so its properties cannot be changed here.</div>}
        >
          <For each={rows()}>{(spec) => <Field scope={props.scope} spec={spec} binding={props.binding} />}</For>
          <AddRow scope={props.scope} binding={props.binding} />
        </Show>
        <div class="pp-foot">
          <button class="pp-done" onClick={closePageProps}>Done</button>
        </div>
      </div>
    </div>
  );
}

function Field(props: { scope: PropsPanelScope; spec: PagePropSpec; binding: Binding }): JSX.Element {
  const initial = readOne(props.scope, props.spec.key) ?? "";
  const write = (value: string | null) => writeOne(props.scope, props.binding, props.spec.key, value);

  if (props.spec.kind === "bool") {
    const [on, setOn] = createSignal(initial.toLowerCase() === "true");
    return (
      <label class="pp-field pp-bool">
        <input
          type="checkbox"
          checked={on()}
          onChange={(e) => {
            if (!stillBound(props.binding)) return;
            setOn(e.currentTarget.checked);
            write(e.currentTarget.checked ? "true" : null);
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
    write(v().trim() || null);
  };
  // A key with no preset is removable; presets clear by emptying the field.
  const removable = !PAGE_PROP_SPECS.some((spec) => spec.key === props.spec.key);
  return (
    <div class="pp-field">
      <div class="pp-row-head">
        <label class="pp-label">{props.spec.label}</label>
        <Show when={removable}>
          <button class="pp-remove" title={`Remove ${props.spec.key}`} onClick={() => write(null)}>Remove</button>
        </Show>
      </div>
      <input
        class="pp-input"
        value={v()}
        placeholder={props.spec.kind === "list" ? "comma, separated" : ""}
        onInput={(e) => setV(e.currentTarget.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.isComposing || e.keyCode === 229) return;
          if (e.key === "Enter") {
            commit();
            closePageProps();
          } else if (e.key === "Escape") {
            if (dismissTopTransient("escape")) e.preventDefault();
          }
        }}
        onBlur={commit}
      />
      <Show when={props.spec.hint}>
        <div class="pp-hint">{props.spec.hint}</div>
      </Show>
    </div>
  );
}

function AddRow(props: { scope: PropsPanelScope; binding: Binding }): JSX.Element {
  const [key, setKey] = createSignal("");
  const [value, setValue] = createSignal("");
  // Validated by the matcher that later has to FIND the key, never a local
  // regex: anything accepted here can be updated and removed. Machine-managed
  // keys are refused — they have their own surfaces.
  const trimmed = () => key().trim();
  const valid = createMemo(() => isEditablePropertyKey(trimmed()) && !machineManaged(trimmed()));
  const commit = () => {
    if (!valid()) return;
    writeOne(props.scope, props.binding, trimmed(), value().trim() || null);
    setKey("");
    setValue("");
  };
  const onKeyDown = (e: KeyboardEvent) => {
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Enter") commit();
    else if (e.key === "Escape" && dismissTopTransient("escape")) e.preventDefault();
  };
  return (
    <div class="pp-field pp-add">
      <label class="pp-label">Add a property</label>
      <div class="pp-add-row">
        <input class="pp-input pp-add-key" value={key()} placeholder="key" onInput={(e) => setKey(e.currentTarget.value)} onKeyDown={onKeyDown} />
        <input class="pp-input pp-add-value" value={value()} placeholder="value" onInput={(e) => setValue(e.currentTarget.value)} onKeyDown={onKeyDown} />
        <button class="pp-add-commit" disabled={!valid()} onClick={commit}>Add</button>
      </div>
      <div class="pp-hint">Any key you like, written to the file as an ordinary property.</div>
    </div>
  );
}
