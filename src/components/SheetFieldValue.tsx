import { For, Match, Switch, createMemo, type JSX } from "solid-js";
import { formatForPage } from "../document";
import { InlineText } from "../render/inline";
import { isFormulaField, type FieldId, type FieldValue } from "../sheet/fields";
import { type FieldType } from "../sheet/config";
import { cellView, isEnumFieldType, type CellView } from "../sheet/cellPresentation";
import type { FormulaValue } from "../sheet/formula";
import { markerLabelClickable } from "../editor/repeat";

/** Render one field value in its declared presentation; read-only and O(value length).
 *  WHAT to show is `cellView` (shared with the static export); this only draws it. */
export function FieldValueView(props: {
  field: FieldId;
  fieldType?: FieldType;
  value: FieldValue | null;
  formulaValue?: FormulaValue | null;
  page: string;
  onControlClick?: (e: MouseEvent) => void;
}): JSX.Element {
  const view = createMemo(() => cellView(props.field, props.fieldType, props.value, props.formulaValue));
  const stopControlDoubleClick = (e: MouseEvent) => {
    if (!props.onControlClick) return;
    e.preventDefault();
    e.stopPropagation();
  };
  // Only an enum chip is a control (it opens the value menu); list and tag chips are inert.
  const enumChip = () => props.field.startsWith("prop:") && isEnumFieldType(props.fieldType);
  const click = () => isFormulaField(props.field) ? undefined : props.onControlClick;
  const dbl = () => isFormulaField(props.field) ? undefined : stopControlDoubleClick;
  const is = <K extends CellView["k"]>(k: K) => {
    const v = view();
    return v.k === k ? (v as Extract<CellView, { k: K }>) : undefined;
  };
  return (
    <Switch>
      <Match when={is("error")}>
        {(v) => <span class="sheet-formula-error" title={v().message}>⚠</span>}
      </Match>
      <Match when={is("marker")}>
        {(v) => (
          <span
            class={`block-marker marker-${v().raw.toLowerCase()}`}
            classList={{ "marker-clickable": markerLabelClickable(v().raw) }}
            onClick={click()}
            onDblClick={dbl()}
          >
            {v().text}
          </span>
        )}
      </Match>
      <Match when={is("priority")}>
        {(v) => <span class={`block-priority priority-${v().raw}`} onClick={click()} onDblClick={dbl()}>{v().text}</span>}
      </Match>
      <Match when={is("date")}>
        {(v) => <span class={`date-chip ${v().cls}`} onClick={click()} onDblClick={dbl()}>{v().text}</span>}
      </Match>
      <Match when={is("chips")}>
        {(v) => (
          <For each={v().values}>
            {(value) => <span class="sheet-tag-chip" onClick={enumChip() ? click() : undefined} onDblClick={enumChip() ? dbl() : undefined}>{value}</span>}
          </For>
        )}
      </Match>
      <Match when={is("check")}>
        {(v) => (
          <input
            class="sheet-checkbox"
            type="checkbox"
            checked={v().checked}
            disabled={isFormulaField(props.field)}
            readOnly
            onClick={click()}
            onDblClick={dbl()}
          />
        )}
      </Match>
      <Match when={is("plain")}>{(v) => <>{v().text}</>}</Match>
      <Match when={is("inline")}>
        {(v) => <InlineText text={v().text} format={formatForPage(props.page)} />}
      </Match>
    </Switch>
  );
}
