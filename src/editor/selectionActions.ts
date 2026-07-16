import { insertLink, toggleInlineFormat, toggleWrap, type Edit, type InlineFormat } from "./format";

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
  apply(
    text: string,
    start: number,
    end: number,
    format?: "md" | "org",
    direction?: "forward" | "backward" | "none",
  ): Edit;
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

const inlineFormat = (
  id: SelectionActionId,
  label: string,
  title: string,
  kind: InlineFormat,
  tier: SelectionAction["tier"] = "essential",
): SelectionAction => ({
  id,
  label,
  title,
  tier,
  apply: (text, start, end, format = "md", direction) =>
    toggleInlineFormat(text, start, end, format, kind, direction),
});

/** Host-owned selection actions. Stable IDs let keybindings, command surfaces,
 * and a future declarative plugin capability refer to the same semantic edits
 * without handing arbitrary DOM/editor access to extensions. */
export const SELECTION_ACTIONS: readonly SelectionAction[] = [
  inlineFormat("bold", "B", "Bold (mod+b)", "bold"),
  inlineFormat("italic", "I", "Italic (mod+i)", "italic"),
  wrap("page-link", "[[ ]]", "Page link", "[[", "essential", "]]"),
  wrap("inline-code", "`", "Inline code", "`"),
  {
    id: "link",
    label: "Link",
    title: "Markdown link",
    tier: "secondary",
    apply: (text, start, end, format = "md") => insertLink(text, start, end, format),
  },
  inlineFormat("strikethrough", "S", "Strikethrough", "strikethrough", "secondary"),
  inlineFormat("highlight", "H", "Highlight", "highlight", "secondary"),
];

export const essentialSelectionActions = SELECTION_ACTIONS.filter((action) => action.tier === "essential");
export const secondarySelectionActions = SELECTION_ACTIONS.filter((action) => action.tier === "secondary");
