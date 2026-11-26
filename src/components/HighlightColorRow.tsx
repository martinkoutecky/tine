import { createSignal, For, Show, type JSX } from "solid-js";
import { BLOCK_COLOR_NAMES } from "../blockColors";
import { SELECTION_ACTIONS, type SelectionAction } from "../editor/selectionActions";
import { setHighlightColor, type Edit } from "../editor/format";
import { selectionActionLabel } from "./SelectionActionLabel";
import { touchPrimary } from "../inputCapability";
import { t } from "../i18n";
import { endTypingBurst } from "../document";

const PLAIN = SELECTION_ACTIONS.find((action) => action.id === "highlight")!;

/** The editor a selection toolbar formats: its page format, whether it shows literal text
 * (a code body or a calculator, which the commit path writes back verbatim), its current
 * text and selection (`null` when it may not be edited now), and how it applies an edit or
 * runs a selection action. */
export type HighlightColorEditor = {
  format(): "md" | "org";
  literal(): boolean;
  read(): { text: string; start: number; end: number; direction?: "forward" | "backward" | "none" } | null;
  apply(edit: Edit): void;
  run(action: SelectionAction): void;
  closeOverflow(): void;
};

/** The highlight colours of one selection toolbar (the textarea's and CodeMirror's), deciding
 * in one place whether the row is offered, what the H button does and what a swatch writes.
 * Colours are offered for Markdown prose only, never for a literal buffer. On desktop the row
 * is always there; on phones H opens and closes it, and it then also carries the plain
 * highlight. A swatch is one Undo step: an edit that changes the text neither joins the typing
 * before it nor the typing after; a press that changes nothing leaves the typing burst alone. */
export function createHighlightColors(editor: HighlightColorEditor) {
  const [open, setOpen] = createSignal(false);
  const colorable = () => editor.format() === "md" && !editor.literal();
  const shown = () => colorable() && (!touchPrimary() || open());
  const pick = (color: string | null) => {
    if (!colorable()) return;
    const selection = editor.read();
    if (!selection) return;
    const edit = setHighlightColor(selection.text, selection.start, selection.end, color, selection.direction);
    if (!edit) return;
    endTypingBurst();
    editor.apply(edit);
    endTypingBurst();
  };
  return {
    /** What a selection action button does: H on a phone toggles the row where colours apply. */
    press(action: SelectionAction) {
      if (action.id !== "highlight" || !touchPrimary() || !colorable()) return editor.run(action);
      editor.closeOverflow();
      setOpen((value) => !value);
    },
    /** The selection is gone: a phone's row closes with it. */
    close: () => setOpen(false),
    Row: (): JSX.Element => (
      <Show when={shown()}>
        <div class="sel-toolbar-colors" role="group" aria-label={t("editor-block-highlight-colors")}>
          <Show when={touchPrimary()}>
            <button type="button" title={PLAIN.title} aria-label={PLAIN.title} data-highlight-plain=""
              onClick={() => editor.run(PLAIN)}>{selectionActionLabel(PLAIN)}</button>
          </Show>
          <For each={BLOCK_COLOR_NAMES}>{(color) => (
            <button type="button" class="sel-color" data-highlight-color={color}
              title={t("editor-block-highlight-color", { color })} aria-label={t("editor-block-highlight-color", { color })}
              style={{ background: `var(--ls-highlight-color-${color})` }} onClick={() => pick(color)} />
          )}</For>
          <button type="button" class="sel-color sel-color-none" data-highlight-color=""
            title={t("editor-block-highlight-no-color")} aria-label={t("editor-block-highlight-no-color")}
            onClick={() => pick(null)}>✕</button>
        </div>
      </Show>
    ),
  };
}
