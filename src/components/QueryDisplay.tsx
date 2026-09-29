import { For, Show, createEffect, createSignal, createUniqueId, onCleanup, type JSX } from "solid-js";
import type { AggFn, ViewSettings } from "../editor/queryIr";
import type { RegistryAccess } from "./querySheetParts";
import { QueryVocabularyPicker, displayFieldEntries, type DisplaySlot } from "./QueryVocabularyPicker";
import { registerTransientLayer } from "../transientLayers";

/** The inline Display panel edits one ViewSettings value. The host owns the
 * guarded write, so a field choice and the header switcher share one save path.
 * Registry reads are provided by the open QueryBuilder. */
export function QueryDisplay(props: {
  view: () => ViewSettings;
  apply: (view: ViewSettings) => void | Promise<boolean>;
  registry: RegistryAccess;
  rowKind: () => "block" | "page";
  formulas?: () => readonly string[];
  parentTransientId?: string;
}): JSX.Element {
  const [open, setOpen] = createSignal(false);
  const [picker, setPicker] = createSignal<DisplaySlot | null>(null);
  const [sampleText, setSampleText] = createSignal("");
  const id = `query-display-${createUniqueId()}`;
  let trigger: HTMLButtonElement | undefined;
  let panel: HTMLDivElement | undefined;
  createEffect(() => {
    if (!open()) return;
    const unregister = registerTransientLayer({ id, parentId: props.parentTransientId,
      root: () => panel ?? null, trigger: () => trigger ?? null,
      dismiss: () => { setOpen(false); return true; } });
    onCleanup(unregister);
  });
  const view = props.view;
  const apply = (patch: Partial<ViewSettings>) => void props.apply({ ...view(), ...patch });
  const openPanel = () => {
    if (!open()) setSampleText(view().sample === undefined ? "" : String(view().sample));
    setOpen(!open());
    setPicker(null);
  };
  const pick = (slot: DisplaySlot, field: string) => {
    if (slot === "sort") apply({ sort: [...view().sort ?? [], [field, "asc"]] });
    if (slot === "column") apply({ columns: [...view().columns ?? [], field] });
    if (slot === "aggregate") apply({ aggregates: [...view().aggregates ?? [], [field, "sum"]] });
    if (slot === "group") apply({ group_by: field });
    setPicker(null);
  };
  const fieldPicker = (slot: DisplaySlot, label: string) => <>
    <button type="button" class="qd-add" onClick={() => setPicker(picker() === slot ? null : slot)}>{label}</button>
    <Show when={picker() === slot}>
      <QueryVocabularyPicker id={`${id}-${slot}`} anchor={props.rowKind()}
        rows={props.registry.rows} pending={props.registry.pending}
        failure={props.registry.failure} onRetry={props.registry.retry}
        entries={(search) => displayFieldEntries({ slot, rowKind: props.rowKind(), rows: props.registry.rows(),
          formulas: props.formulas?.(), search })}
        onPick={(choice) => { if (choice.kind === "field") pick(slot, choice.field); }} />
    </Show>
  </>;
  const move = <T,>(items: T[], from: number, to: number): T[] => {
    const copy = [...items];
    if (to < 0 || to >= copy.length) return copy;
    copy.splice(to, 0, copy.splice(from, 1)[0]);
    return copy;
  };
  const listRow = (label: string, index: number, length: number, moveRow: (to: number) => void, remove: () => void, extra?: JSX.Element) =>
    <div class="qd-row"><span>{label}</span>{extra}
      <button type="button" aria-label={`Move ${label} up`} disabled={index === 0} onClick={() => moveRow(index - 1)}>↑</button>
      <button type="button" aria-label={`Move ${label} down`} disabled={index === length - 1} onClick={() => moveRow(index + 1)}>↓</button>
      <button type="button" aria-label={`Remove ${label}`} onClick={remove}>×</button>
    </div>;
  const sample = () => {
    const text = sampleText().trim();
    if (text === "") return { valid: true, value: undefined };
    if (!/^\d+$/.test(text) || Number(text) > 4294967295) return { valid: false, value: undefined };
    return { valid: true, value: Number(text) };
  };
  const commitSample = () => {
    const parsed = sample();
    if (parsed.valid && parsed.value !== view().sample) apply({ sample: parsed.value });
  };
  return <div class="qd-root">
    <button ref={trigger} type="button" class="qd-trigger" aria-expanded={open()} aria-controls={id}
      onClick={openPanel}>Display</button>
    <Show when={open()}><div ref={panel} id={id} class="qd-panel" role="dialog" aria-label="Query display settings"
      onClick={(event) => event.stopPropagation()}>
      <section><h4>View</h4><div role="group" aria-label="Query view">
        <For each={["search", "list", "table", "board"] as const}>{(kind) =>
          <button type="button" classList={{ active: (view().view ?? "list") === kind }}
            onClick={() => apply({ view: kind })}>{kind}</button>}</For>
      </div></section>
      <section><h4>Group by</h4><span>{view().group_by || "No grouping"}</span>
        <button type="button" onClick={() => apply({ group_by: "" })}>None</button>
        {fieldPicker("group", "Change")}
      </section>
      <section><h4>Sort</h4>
        <For each={view().sort ?? []}>{([field, dir], index) => listRow(field, index(), view().sort?.length ?? 0,
          (to) => apply({ sort: move(view().sort ?? [], index(), to) }),
          () => apply({ sort: (view().sort ?? []).filter((_, i) => i !== index()) }),
          <button type="button" aria-label={`Sort ${field} ${dir === "asc" ? "descending" : "ascending"}`}
            onClick={() => apply({ sort: (view().sort ?? []).map(([f, d], i) => i === index() ? [f, d === "asc" ? "desc" : "asc"] : [f, d]) })}>
            {dir === "asc" ? "Asc" : "Desc"}</button>)}
        </For>{fieldPicker("sort", "+ sort")}
      </section>
      <section><h4>Columns</h4>
        <Show when={!view().columns?.length}><div>All fields the rows carry</div></Show>
        <For each={view().columns ?? []}>{(field, index) => listRow(field, index(), view().columns?.length ?? 0,
          (to) => apply({ columns: move(view().columns ?? [], index(), to) }),
          () => apply({ columns: (view().columns ?? []).filter((_, i) => i !== index()) }))}</For>
        {fieldPicker("column", "+ column")}
      </section>
      <section><h4>Summarize</h4>
        <For each={view().aggregates ?? []}>{([field, fn], index) => listRow(field || "Whole result", index(), view().aggregates?.length ?? 0,
          (to) => apply({ aggregates: move(view().aggregates ?? [], index(), to) }),
          () => apply({ aggregates: (view().aggregates ?? []).filter((_, i) => i !== index()) }),
          <button type="button" onClick={() => {
            const cycle: AggFn[] = ["count", "sum", "avg"];
            apply({ aggregates: (view().aggregates ?? []).map(([f, a], i) => i === index()
              ? [f, cycle[(cycle.indexOf(a) + 1) % cycle.length]] : [f, a]) });
          }}>{fn}</button>)}
        </For>
        <button type="button" onClick={() => apply({ aggregates: [...view().aggregates ?? [], ["", "count"]] })}>+ count</button>
        {fieldPicker("aggregate", "+ property")}
      </section>
      <section><h4>Sample</h4><input aria-label="Sample size" inputmode="numeric" placeholder="No limit"
        value={sampleText()} onInput={(event) => setSampleText(event.currentTarget.value)}
        onBlur={commitSample} onKeyDown={(event) => { if (event.key === "Enter") commitSample(); }} />
        <Show when={!sample().valid}><div role="alert">A sample is a whole number up to 4294967295.</div></Show>
        <Show when={sample().valid && sample().value === 0}><div>No results (sample 0)</div></Show>
      </section>
      <button type="button" onClick={() => setOpen(false)}>Done</button>
    </div></Show>
  </div>;
}
