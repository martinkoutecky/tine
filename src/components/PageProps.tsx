import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, untrack, type JSX } from "solid-js";
import { pagePropsPanel, closePageProps, type PropsPanelScope } from "../ui";
import {
  blockPageReadOnly, blockProperty, formatForBlock, node, pageByName,
  readPageProperties, readPageProperty, setBlockProperty, setPageProperty,
} from "../document";
import { PAGE_PROP_SPECS, isEditablePropertyKey, isSheetCellHidden, type PagePropSpec } from "../editor/properties";
import { facetsOf } from "../render/facets";
import { dismissTopTransient, registerTransientLayer } from "../transientLayers";
import { stillBound, type Binding } from "../binding";
import { pushToast } from "../toasts";
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

// The loaded page/block object the panel opened on. A genuine reload (external
// change, delete + recreate, graph reset) replaces it — even under the same page
// name or block id — so comparing it binds every write to what the user saw.
const subjectOf = (scope: PropsPanelScope): object | undefined =>
  scope.kind === "page" ? pageByName(scope.name) : node(scope.id);

/** The one write door of the panel. A write lands only while the graph session
 *  (`binding`) AND the subject instance captured at open are both current;
 *  otherwise it is refused visibly and the panel closes, so a typed edit is
 *  never dropped silently and never overwrites an intervening change. */
function writeOne(scope: PropsPanelScope, binding: Binding, subject: object | undefined, key: string, value: string | null): boolean {
  const stale = !stillBound(binding) ? "The graph changed" : !subject || subjectOf(scope) !== subject
    ? `This ${scope.kind} changed or was reloaded` : null;
  if (stale) {
    pushToast(`${stale} while its properties panel was open, so "${key}" was not saved. Reopen the panel to edit it.`, "error");
    closePageProps();
    return false;
  }
  if (scope.kind === "page") setPageProperty(scope.name, key, value);
  else setBlockProperty(scope.id, key, value);
  return true;
}

/** Writable only when the subject is loaded and not read-only. An unknown or
 *  no-longer-loaded page/block shows the read-only notice, never an edit row. */
function scopeWritable(scope: PropsPanelScope): boolean {
  if (scope.kind === "block") return !!node(scope.id) && !blockPageReadOnly(scope.id);
  const page = pageByName(scope.name);
  return !!page && !page.readOnly && !page.guide;
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
  const subject = untrack(() => subjectOf(props.scope));
  const write = (key: string, value: string | null) => writeOne(props.scope, props.binding, subject, key, value);
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
          fallback={<div class="pp-hint">This {props.scope.kind} is read-only or no longer loaded, so its properties cannot be changed here.</div>}
        >
          <For each={rows()}>{(spec) => <Field scope={props.scope} spec={spec} write={write} />}</For>
          <AddRow scope={props.scope} write={write} />
        </Show>
        <div class="pp-foot">
          <button class="pp-done" onClick={closePageProps}>Done</button>
        </div>
      </div>
    </div>
  );
}

type Write = (key: string, value: string | null) => boolean;

function Field(props: { scope: PropsPanelScope; spec: PagePropSpec; write: Write }): JSX.Element {
  // The value last written or read; only a real local edit writes (see commit).
  let saved = readOne(props.scope, props.spec.key) ?? "";
  const write = (value: string | null) => props.write(props.spec.key, value);

  if (props.spec.kind === "bool") {
    const [on, setOn] = createSignal(saved.toLowerCase() === "true");
    return (
      <label class="pp-field pp-bool">
        <input
          type="checkbox"
          checked={on()}
          onChange={(e) => {
            const checked = e.currentTarget.checked;
            if (write(checked ? "true" : null)) setOn(checked);
            else e.currentTarget.checked = on();
          }}
        />
        <span class="pp-text">
          <span class="pp-label">{props.spec.label}</span>
          <span class="pp-hint">{props.spec.hint}</span>
        </span>
      </label>
    );
  }

  const [v, setV] = createSignal(saved);
  // Only write on an actual local edit. Otherwise blurring/closing the panel
  // re-commits the value read when it opened — clobbering a concurrent external
  // edit (OG/Syncthing) that the file-watcher reloaded while the panel was open.
  const commit = () => {
    if (v() === saved) return;
    if (write(v().trim() || null)) saved = v();
  };
  // A key with no preset is removable; presets clear by emptying the field. A
  // A key outside the editable grammar is shown
  // read-only: rewriting it would keep a key some Tine readers cannot see.
  const removable = !PAGE_PROP_SPECS.some((spec) => spec.key === props.spec.key);
  const editable = isEditablePropertyKey(props.spec.key);
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
        disabled={!editable}
        title={editable ? undefined : "Tine can remove this key but not rewrite it: keys may use letters, numbers, _, ., / or -."}
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

function AddRow(props: { scope: PropsPanelScope; write: Write }): JSX.Element {
  const [key, setKey] = createSignal("");
  const [value, setValue] = createSignal("");
  // isEditablePropertyKey is the grammar every Tine reader finds again, so an
  // added key can be updated and removed. Machine-managed keys have their own
  // surfaces; an existing key is edited on its own row (the add-row would
  // silently replace a value the user can see above).
  const trimmed = () => key().trim();
  const refusal = createMemo(() => {
    const k = trimmed();
    if (!k) return null;
    if (!isEditablePropertyKey(k)) return "Keys may use only letters, numbers, _, ., / or -, so every Tine reader finds them again.";
    if (machineManaged(k)) return `"${k}" is managed by Tine and cannot be set here.`;
    if (readOne(props.scope, k) !== null) return `"${k}" already exists; edit its row above.`;
    return null;
  });
  const valid = () => !!trimmed() && !refusal();
  const commit = () => {
    if (!valid() || !props.write(trimmed(), value().trim() || null)) return;
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
      <Show when={refusal()} fallback={<div class="pp-hint">Any key you like, written to the file as an ordinary property.</div>}>
        <div class="pp-hint pp-error">{refusal()}</div>
      </Show>
    </div>
  );
}
