import { Show } from "solid-js";
import { t } from "../i18n";
import { authorMarksHidden } from "../authorMarksPref";

/** A decoration in the existing gap, also a keyboard/hover author disclosure.
 * No edit gesture: selecting/focusing the mark never moves the block's caret. */
export function AuthorGutter(props: { author: string | null }) {
  return <Show when={props.author !== null && !authorMarksHidden()}>
    <span class="author-mark" role="img" tabIndex={0}
      aria-label={t("comment-written-by", { author: props.author! })}
      onMouseDown={event => event.stopPropagation()}>
      <span class="author-mark-label">{props.author}</span>
    </span>
  </Show>;
}
