import { For, Show, type JSX } from "solid-js";
import { BLOCK_COLOR_NAMES } from "../blockColors";
import { SELECTION_ACTIONS, type SelectionAction } from "../editor/selectionActions";
import { selectionActionLabel } from "./SelectionActionLabel";
import { t } from "../i18n";
import { endTypingBurst } from "../document";

const PLAIN = SELECTION_ACTIONS.find((action) => action.id === "highlight")!;

/** The selection toolbar's row of highlight colours, the block context
 * menu's colours as Logseq's theme variables, then "no colour". Shared by the textarea and
 * CodeMirror toolbars; `pick` applies `setHighlightColor` synchronously. `plain`, given on phones where
 * the row opens from the H button, runs the plain highlight action from the row's start. */
export function HighlightColorRow(props: { pick(color: string | null): void; plain?: (action: SelectionAction) => void }): JSX.Element {
  // One press is one Undo step: the edit neither joins the typing before it nor the typing after.
  const pick = (color: string | null) => { endTypingBurst(); props.pick(color); endTypingBurst(); };
  return (
    <div class="sel-toolbar-colors" role="group" aria-label={t("editor-block-highlight-colors")}>
      <Show when={props.plain}>{(plain) => (
        <button type="button" title={PLAIN.title} aria-label={PLAIN.title} data-highlight-plain=""
          onClick={() => plain()(PLAIN)}>{selectionActionLabel(PLAIN)}</button>
      )}</Show>
      <For each={BLOCK_COLOR_NAMES}>{(color) => (
        <button type="button" class="sel-color" data-highlight-color={color}
          title={t("editor-block-highlight-color", { color })} aria-label={t("editor-block-highlight-color", { color })}
          style={{ background: `var(--ls-highlight-color-${color})` }} onClick={() => pick(color)} />
      )}</For>
      <button type="button" class="sel-color sel-color-none" data-highlight-color=""
        title={t("editor-block-highlight-no-color")} aria-label={t("editor-block-highlight-no-color")}
        onClick={() => pick(null)}>✕</button>
    </div>
  );
}
