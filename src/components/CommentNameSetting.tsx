import { createSignal, type JSX } from "solid-js";
import { msg, t } from "../i18n";
import { Rich } from "../i18n/Rich";
import { commentAuthorName, setCommentAuthorName } from "../commentIdentity";
import { Field } from "./settingsField";

/** Settings → Editing: "Your name", the `author::` your margin comments and
 * replies carry (commentIdentity.ts). Saved when the field loses focus or on
 * Enter, not per keystroke; empty writes no author. */
export function CommentNameSetting(): JSX.Element {
  const [draft, setDraft] = createSignal<string | null>(null);
  const commit = () => {
    const value = draft();
    setDraft(null);
    if (value !== null) setCommentAuthorName(value);
  };
  return (
    <Field label={t("comment-name-label")} hint={<Rich message={msg("comment-name-hint")} />}>
      <input
        class="settings-input"
        aria-label={t("comment-name-label")}
        placeholder={t("comment-name-placeholder")}
        maxLength={80}
        value={draft() ?? commentAuthorName()}
        onInput={(event) => setDraft(event.currentTarget.value)}
        onChange={commit}
        onBlur={commit}
        onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
      />
    </Field>
  );
}
