import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { linkSchemeRequest, type LinkSchemeRequest } from "../linkSchemePrompt";
import { registerTransientLayer } from "../transientLayers";
import { t } from "../i18n";
import { DialogActions } from "./DialogActions";
import "./LinkSchemePrompt.css";

/** The same explicit consent on desktop and mobile. Cancel, Escape, Back and
 * outside clicks never save the checkbox choice or open the target. */
export function LinkSchemePrompt() {
  return <Show when={linkSchemeRequest()} keyed>{request => <Prompt request={request} />}</Show>;
}

function Prompt(props: { request: LinkSchemeRequest }) {
  const request = props.request;
  const [always, setAlways] = createSignal(false);
  let root!: HTMLFormElement;
  let cancel!: HTMLButtonElement;
  const dismiss = () => request.finish("cancel");
  onMount(() => {
    const previous = root.ownerDocument.activeElement as HTMLElement | null;
    cancel.focus();
    onCleanup(registerTransientLayer({ id: "link-scheme", root: () => root, dismiss: () => { dismiss(); return true; } }));
    onCleanup(() => { dismiss(); if (previous?.isConnected) previous.focus(); });
  });
  return <div class="modal-overlay graph-name-overlay" onClick={dismiss}>
    <form ref={root} class="export-modal graph-name-prompt link-scheme-prompt" role="dialog" aria-modal="true"
      aria-label={t("links-open-custom-scheme-title", { scheme: request.scheme })}
      onClick={event => event.stopPropagation()}
      onSubmit={event => { event.preventDefault(); request.finish(always() ? "always" : "open"); }}>
      <div class="export-head">{t("links-open-custom-scheme-title", { scheme: request.scheme })}</div>
      <p class="link-scheme-target">{t("links-open-custom-scheme-confirm", { scheme: request.scheme, url: request.url })}</p>
      <label><input type="checkbox" checked={always()} onChange={event => setAlways(event.currentTarget.checked)} />
        {t("links-always-open-scheme", { scheme: request.scheme })}
      </label>
      <p>{t("links-consent-device-hint")}</p>
      <DialogActions cancel={<button ref={cancel} type="button" class="export-btn-secondary" onClick={dismiss}>{t("confirm-cancel")}</button>}
        primary={<button type="submit" class="export-btn-primary">{t("confirm-open-link")}</button>} />
    </form>
  </div>;
}
