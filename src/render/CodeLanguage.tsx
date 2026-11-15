import { Show, type JSX } from "solid-js";
import { t } from "../i18n";

/** The shared reading/editing label. Activation reveals the existing fence
 * source; the caller owns its editor and caret. The placeholder is display only.
 * This pointer route intentionally extends Logseq (D-20261113 CODE-879). */
/** Focus moving from an editor to its own block's label keeps the edit: the label is
 * part of that session, and keyboard activation lands back in the editor. */
export function blurToOwnCodeLanguage(target: EventTarget | null, editor: Element): boolean {
  const element = target as Element | null;
  return !!element?.matches?.("button.code-language") && element.closest("[data-block-id]") === editor.closest("[data-block-id]");
}

export function CodeLanguage(props: { lang: string; onEdit?: () => void }): JSX.Element {
  const label = () => props.lang.toLowerCase() || t("render-code-plain");
  return <Show when={props.onEdit} fallback={<span class="code-language">{label()}</span>}>
    <button type="button" class="code-language" aria-label={t("render-edit-code-language")}
      title={t("render-edit-code-language")}
      onMouseDown={event => event.preventDefault()}
      onClick={event => { event.stopPropagation(); props.onEdit?.(); }}>{label()}</button>
  </Show>;
}
