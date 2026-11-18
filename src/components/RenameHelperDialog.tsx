import { For, Show } from "solid-js";
import { t } from "../i18n";
import { refuseStaleWrite } from "../binding";
import { renameHelperRequest, type RenameHelperRequest } from "../renameHelperDialogs";
import { DialogActions } from "./DialogActions";
import { RenameDialogFrame } from "./RenameDialogFrame";

/** The day-move and contested-name dialogs (D-20261115-HELP): what Tine knows
 *  about the refused rename, and the legal steps it offers instead. Everything
 *  shown is the backend's answer (`rename_offer`); the steps are performed by
 *  `renameOrMergePage`. */
export function RenameHelperDialog() {
  return <Show when={renameHelperRequest()} keyed>{(request) => request.kind === "dayMove"
    ? <DayMove request={request} /> : <Contested request={request} />}</Show>;
}

const listed = (names: readonly string[], count: number) =>
  count > names.length ? t("rename-alias-dialog-more", { pages: names.join(", "), more: count - names.length }) : names.join(", ");

/** The open question is still `request` (a graph switch or a newer question
 *  closes it); otherwise the click is refused as stale. */
function current(request: RenameHelperRequest): boolean {
  if (renameHelperRequest() !== request) { refuseStaleWrite(t("rename-alias-dialog-stale-what")); return false; }
  return true;
}

function DayMove(props: { request: Extract<RenameHelperRequest, { kind: "dayMove" }> }) {
  const request = props.request;
  const offer = request.offer;
  const day = offer.day.name, name = offer.name;
  const title = t("rename-day-dialog-title", { day, name });
  const choice = offer.into ? t("rename-day-dialog-move-into", { name }) : t("rename-day-dialog-move", { name });
  return (
    <RenameDialogFrame id="tine-rename-day-dialog" label={title} dismiss={() => request.finish(null)}>
      <h2>{title}</h2>
      <ul data-day-move>
        <li data-day-move-blocks>{offer.into ? t("rename-day-dialog-into", { name }) : t("rename-day-dialog-new", { name })}</li>
        <li data-day-move-keeps>{t("rename-day-dialog-keeps", { day, name })}</li>
        <Show when={offer.day.aliases.length}><li data-day-move-alias>{t("rename-day-dialog-alias", { aliases: offer.day.aliases.join(", ") })}</li></Show>
        <li data-day-move-references>{offer.referenceCount
          ? t("rename-day-dialog-references", { day, count: offer.referenceCount, pages: listed(offer.references, offer.referenceCount) })
          : t("rename-day-dialog-no-references", { day })}</li>
        <Show when={offer.retargeted}>{(link) =>
          <li data-retarget={link().name}>{t("rename-alias-dialog-retarget", { name: link().name, before: link().before, after: link().after, count: link().pageCount, pages: listed(link().pages, link().pageCount) })}</li>}</Show>
      </ul>
      <DialogActions
        cancel={<button type="button" data-choice-cancel onClick={() => request.finish(null)}>{t("confirm-cancel")}</button>}
        primary={<button type="button" data-choice="move" onClick={() => current(request) && request.finish(true)}>{choice}</button>}
      />
    </RenameDialogFrame>
  );
}

function Contested(props: { request: Extract<RenameHelperRequest, { kind: "contested" }> }) {
  const request = props.request;
  const offer = request.offer;
  const title = t("rename-contested-dialog-title", { name: offer.name });
  return (
    <RenameDialogFrame id="tine-rename-contested-dialog" label={title} dismiss={() => request.finish(null)}>
      <h2>{title}</h2>
      <p>{t("rename-contested-dialog-intro", { from: request.from, to: request.to })}</p>
      <ul class="tine-rename-alias-component">
        <For each={offer.files}>{(file) =>
          <li data-contested-file={file}>
            <code>{file}</code>
            <Show when={file === offer.opens}><span class="tine-rename-alias-tag">{t("rename-alias-dialog-opens-now", { to: offer.name })}</span></Show>
            <button type="button" data-choice="keep" data-keep={file} onClick={() => current(request) && request.finish(file)}>{t("rename-contested-dialog-keep", { file })}</button>
          </li>}</For>
      </ul>
      <DialogActions cancel={<button type="button" data-choice-cancel onClick={() => request.finish(null)}>{t("confirm-cancel")}</button>} primary={null} />
    </RenameDialogFrame>
  );
}
