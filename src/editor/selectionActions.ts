import { insertLink, toggleWrap, type Edit } from "./format";

export type SelectionActionId =
  | "bold"
  | "italic"
  | "page-link"
  | "inline-code"
  | "link"
  | "strikethrough"
  | "highlight";

export interface SelectionAction {
  id: SelectionActionId;
  label: string;
  title: string;
  tier: "essential" | "secondary";
  apply(text: string, start: number, end: number, format?: "md" | "org"): Edit;
}

const wrap = (
  id: SelectionActionId,
  label: string,
  title: string,
  left: string,
  tier: SelectionAction["tier"] = "essential",
  right = left,
): SelectionAction => ({
  id,
  label,
  title,
  tier,
  apply: (text, start, end) => toggleWrap(text, start, end, left, right),
});

/** Host-owned selection actions. Stable IDs let keybindings, command surfaces,
 * and a future declarative plugin capability refer to the same semantic edits
 * without handing arbitrary DOM/editor access to extensions. */
export const SELECTION_ACTIONS: readonly SelectionAction[] = [
  wrap("bold", "B", "Bold (mod+b)", "**"),
  wrap("italic", "I", "Italic (mod+i)", "*"),
  wrap("page-link", "[[ ]]", "Page link", "[[", "essential", "]]"),
  wrap("inline-code", "`", "Inline code", "`"),
  {
    id: "link",
    label: "Link",
    title: "Markdown link",
    tier: "secondary",
    apply: (text, start, end, format = "md") => insertLink(text, start, end, format),
  },
  wrap("strikethrough", "S", "Strikethrough", "~~", "secondary"),
  wrap("highlight", "H", "Highlight", "==", "secondary"),
];

export const essentialSelectionActions = SELECTION_ACTIONS.filter((action) => action.tier === "essential");
export const secondarySelectionActions = SELECTION_ACTIONS.filter((action) => action.tier === "secondary");
