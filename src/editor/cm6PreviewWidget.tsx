import { runWithOwner, type Owner } from "solid-js";
import { render } from "solid-js/web";
import { EditorView, WidgetType } from "@codemirror/view";
import { PageRef, renderInlines } from "../render/inline";
import { renderBlocks } from "../render/body";
import { renderedClickOffset } from "../components/blockGestures";
import { CalGlyph } from "../components/blockParts";
import { taskCheckboxState } from "../markers";
import type { PreviewSpan } from "./cm6Preview";

/** The marker as the view draws it (Rendered.tsx): the task checkbox and the marker chip are SIBLINGS joined by a space
 * (the checkbox colour is `.block-task-checkbox:has(+ .marker-x)`, and the chip must not wrap the box), and CANCELED has no box.
 * The pointer reveals the source instead of toggling. */
function TaskMarker(props: { label: string }) {
  const state = taskCheckboxState(props.label);
  return <>
    {state !== null && <><span class="block-task-checkbox" classList={{ checked: state }} aria-hidden="true" />{" "}</>}
    <span class={`block-marker marker-${props.label.toLowerCase()}`}>{props.label}</span>
  </>;
}
/** The priority chip with the view's text: the span covers `[#A]` and the view shows it whole. */
function PriorityChip(props: { label: string }) {
  return <span class={`block-priority priority-${props.label}`}>[#{props.label}]</span>;
}

/** Reuse the read renderer, with a disposable Solid owner for each CM widget.
 * A pointer edits the host source instead of following a link or mutating a
 * nested rendered control. This boundary is shared by every construct. */
export class PreviewWidget extends WidgetType {
  private dispose?: () => void;
  constructor(readonly span: PreviewSpan, readonly raw: string, readonly format: "md" | "org", readonly owner: Owner | null, readonly blockId: string) { super(); }
  eq(other: PreviewWidget) {
    return this.raw === other.raw && this.format === other.format && this.blockId === other.blockId
      && this.span.from === other.span.from && this.span.to === other.span.to && this.span.kind === other.span.kind;
  }
  toDOM(view: EditorView) {
    const el = view.dom.ownerDocument.createElement("span");
    el.className = "cm6-preview";
    el.dataset.cm6Construct = this.span.kind;
    const { span, format, blockId } = this;
    this.dispose = runWithOwner(this.owner, () => render(() => {
      // The view's own date chip (Rendered.tsx). The pointer reveals the source instead of opening the date picker.
      if (span.planning) return <span class={`date-chip ${span.planning.ts === "Deadline" ? "deadline" : "scheduled"}`}><CalGlyph /> {span.planning.value}</span>;
      if (span.inline) return renderInlines([span.inline], blockId, true, false, format);
      if (span.block) {
        const header = span.block.kind === "bullet" || span.block.kind === "heading" ? span.block : null;
        return <>
          {header?.marker && <TaskMarker label={header.marker} />}
          {header?.priority && <PriorityChip label={header.priority} />}
          {renderBlocks([span.block], blockId, span.block.kind === "bullet" ? span.block.size : undefined, false, format)}
          {header?.htags?.map(name => <PageRef name={name} blockId={blockId} tag />)}
        </>;
      }
      if (span.headerTags) return span.headerTags.map(name => <PageRef name={name} blockId={blockId} tag />);
      if (span.kind === "task") return <TaskMarker label={span.label!} />;
      return <PriorityChip label={span.label!} />;
    }, el));
    const reveal = (event: MouseEvent | PointerEvent) => {
      if (event.button !== 0) return;
      const lo = span.revealFrom ?? span.from;
      const offset = renderedClickOffset(el, this.raw, format, event) ?? lo;
      const head = Math.max(lo, Math.min(span.to, offset));
      event.preventDefault(); event.stopPropagation();
      view.dispatch({ selection: { anchor: event.shiftKey ? view.state.selection.main.anchor : head, head }, scrollIntoView: true });
      view.focus();
    };
    // Touch/pen do not necessarily emit mousedown. Mouse uses mousedown so
    // the source is exposed before the browser attempts a DOM selection.
    el.addEventListener("pointerdown", event => { if (event.pointerType !== "mouse") reveal(event); }, true);
    el.addEventListener("mousedown", reveal, true);
    el.addEventListener("click", event => { event.preventDefault(); event.stopPropagation(); }, true);
    return el;
  }
  destroy() { this.dispose?.(); this.dispose = undefined; }
  ignoreEvent() { return true; }
}
