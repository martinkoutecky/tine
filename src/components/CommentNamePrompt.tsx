import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { commentNameRequest, type CommentNameRequest } from "../commentNamePrompt";
import { registerTransientLayer } from "../transientLayers";
import { refuseStaleWrite } from "../binding";
import { t } from "../i18n";
import { DialogActions } from "./DialogActions";

/** The one-time "Your name" question before the first margin comment or reply in
 * a git graph (commentNamePrompt.ts, tine#841). Saving a name keeps it for every
 * graph on this device; Skip, Escape and a click outside decline for good. The
 * held comment is created either way once the dialog closes. */
export function CommentNamePrompt() {
  return <Show when={commentNameRequest()}>{request => <NameForm request={request()} />}</Show>;
}

function NameForm(props: { request: CommentNameRequest }) {
  const request = props.request;
  const [name, setName] = createSignal("");
  let root!: HTMLFormElement;
  let input!: HTMLInputElement;
  // A graph switch clears the request (and says so); a late click must not write into the new graph.
  const finish = (name: string | null) => {
    if (commentNameRequest() !== request) return refuseStaleWrite(t("comment-stale-write-what"));
    request.finish(name);
  };
  const dismiss = () => finish(null);
  onMount(() => {
    input.focus();
    onCleanup(registerTransientLayer({ id: "comment-name", root: () => root, dismiss: () => { dismiss(); return true; } }));
  });
  const submit = (event: SubmitEvent) => {
    event.preventDefault();
    finish(name());
  };
  return <div class="modal-overlay graph-name-overlay" onClick={dismiss}>
    <form ref={root} class="export-modal graph-name-prompt comment-name-prompt" role="dialog" aria-modal="true"
      aria-label={t("comment-name-prompt-title")} onClick={event => event.stopPropagation()} onSubmit={submit}>
      <div class="export-head">{t("comment-name-prompt-title")}</div>
      <p class="comment-name-prompt-body">{t("comment-name-prompt-body")}</p>
      <label class="graph-name-field">{t("comment-name-label")}<input ref={input} value={name()} maxLength={80}
          placeholder={t("comment-name-placeholder")} aria-label={t("comment-name-label")}
          onInput={event => setName(event.currentTarget.value)} />
      </label>
      <p class="comment-name-prompt-scope">{t("comment-name-prompt-scope")}</p>
      <DialogActions
        cancel={<button type="button" class="export-btn-secondary" onClick={dismiss}>{t("comment-name-prompt-dismiss")}</button>}
        primary={<button type="submit" class="export-btn-primary" disabled={!name().trim()}>{t("comment-name-prompt-save")}</button>} />
    </form>
  </div>;
}
