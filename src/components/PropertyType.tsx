import { For, Show, createSignal, type Accessor, type JSX } from "solid-js";
import { ensurePagePropertyOnKeyPage } from "../document";
import { pushToast } from "../toasts";
import type { Cardinality, ObservedType, RegistryRow } from "../editor/queryIr";
import { registryRowFor } from "../editor/queryPropertyType";

export const DECLARED_TYPE_KEY = "tine.type";
export const DECLARABLE_TYPES: readonly ObservedType[] = ["text", "number", "date", "checkbox", "ref"];

/** The exact declaration grammar read by the registry. O(1), pure. */
export function declarationValue(type: ObservedType, cardinality: Cardinality): string {
  return cardinality === "many" ? `list of ${type}` : type;
}

/** Shows the registry's observed and declared types and writes a declaration on
 * the normalized key page. Save failures are reported and retain the menu. */
export function PropertyType(props: {
  propertyKey: string;
  rows: Accessor<RegistryRow[] | undefined>;
  onDeclarationWritten: () => void;
  readOnly?: boolean;
}): JSX.Element {
  const row = () => registryRowFor(props.rows(), props.propertyKey);
  const [open, setOpen] = createSignal(false);
  const [many, setMany] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const phrase = (type: ObservedType, cardinality: Cardinality) => declarationValue(type, cardinality);
  const write = async (value: string | null) => {
    const current = row();
    if (!current || busy()) return;
    setBusy(true);
    try {
      await ensurePagePropertyOnKeyPage(current.normalized_name, DECLARED_TYPE_KEY, value);
      props.onDeclarationWritten();
      setOpen(false);
    } catch (error) {
      pushToast(`Could not save property type: ${error instanceof Error ? error.message : String(error)}`, "error");
    } finally {
      setBusy(false);
    }
  };
  return <span class="qb-prop-type" onClick={(event) => event.stopPropagation()}>
    <Show when={row()} fallback={<Show when={props.rows()}><span>no observed values</span></Show>}>
      {(current) => <>
        <span class="qb-prop-type-badge">
          {current().declared ? phrase(...current().declared!) : phrase(current().observed_type, current().cardinality)}
          <small> ({current().declared ? "declared" : "observed"})</small>
        </span>
        <Show when={current().declared && current().mismatch_count > 0}>
          <span class="qb-prop-type-mismatch">{current().mismatch_count} blocks or pages do not match</span>
        </Show>
        <Show when={!props.readOnly}>
          <button type="button" disabled={busy()} onClick={() => setOpen(!open())}>declare type…</button>
          <Show when={open()}><span class="qb-prop-type-menu">
            <label><input type="checkbox" checked={many()} onChange={(event) => setMany(event.currentTarget.checked)} /> list of</label>
            <For each={DECLARABLE_TYPES}>{(type) => <button type="button"
              onClick={() => void write(declarationValue(type, many() ? "many" : "one"))}>
              {declarationValue(type, many() ? "many" : "one")}
            </button>}</For>
            <Show when={current().declared}><button type="button" onClick={() => void write(null)}>remove declaration</button></Show>
          </span></Show>
        </Show>
      </>}
    </Show>
  </span>;
}
