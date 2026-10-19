import type { JSX } from "solid-js";
import type { SelectionAction } from "../editor/selectionActions";

/** The button face of a selection action, shared by the editor's selection
 *  toolbar and the rendered-selection toolbar (GH #375). */
export function selectionActionLabel(action: SelectionAction): JSX.Element {
  if (action.id === "bold") return <b>{action.label}</b>;
  if (action.id === "italic") return <i>{action.label}</i>;
  if (action.id === "strikethrough") return <s>{action.label}</s>;
  if (action.id === "highlight") return <mark>{action.label}</mark>;
  if (action.id === "link") return (
    <svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true">
      <path d="M9 15l6-6M10 6l1-1a4 4 0 015.7 5.7l-1 1M14 18l-1 1a4 4 0 01-5.7-5.7l1-1"
        fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
    </svg>
  );
  return action.label;
}
