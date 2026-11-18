import { For, Show } from "solid-js";
import { render, t } from "../i18n";
import { refuseStaleWrite } from "../binding";
import { aliasDialogRequest, type AliasDialogAnswer, type AliasDialogRequest } from "../renameAliasDialog";
import { renameRefusalMessage } from "../renameRefusal";
import type { AliasDialogOffer, AliasDialogPage, RenameEffect } from "../types";
import { DialogActions } from "./DialogActions";
import { RenameDialogFrame } from "./RenameDialogFrame";

/** The alias dialog (D-20261106-R13-UI, D-20261115-HELP): the typed name is
 *  only another page's alias, which the model refuses (r6 O3). It shows what
 *  Tine knows (the pages that declare the name, their alias lines, what the
 *  name opens now) and offers each request the backend says the model allows,
 *  with what that request changes, including links that will open another page
 *  with their text unchanged. Everything shown is the backend's answer
 *  (`rename_offer`); a choice is answered here and performed by the rename. */
export function RenameAliasDialog() {
  return <Show when={aliasDialogRequest()}>{(request) => <Dialog request={request()} />}</Show>;
}

const listed = (names: readonly string[], count: number) =>
  count > names.length ? t("rename-alias-dialog-more", { pages: names.join(", "), more: count - names.length }) : names.join(", ");

function Effect(props: { effect: RenameEffect }) {
  return (
    <>
      <Show when={props.effect.pageCount > 0}>
        <li>{t("rename-alias-dialog-pages", { count: props.effect.pageCount, pages: listed(props.effect.pages, props.effect.pageCount) })}</li>
      </Show>
      <For each={props.effect.retargeted}>{(link) =>
        <li data-retarget={link.name}>{t("rename-alias-dialog-retarget", { name: link.name, before: link.before, after: link.after, count: link.pageCount, pages: listed(link.pages, link.pageCount) })}</li>
      }</For>
    </>
  );
}

function PageLine(props: { page: AliasDialogPage; to: string; opens: boolean; source: boolean }) {
  return (
    <li data-component-page={props.page.name}>
      <strong>{props.page.name}</strong>
      <Show when={props.page.aliases.length}><code>{t("rename-alias-dialog-aliases", { aliases: props.page.aliases.join(", ") })}</code></Show>
      <Show when={props.opens}><span class="tine-rename-alias-tag">{t("rename-alias-dialog-opens-now", { to: props.to })}</span></Show>
      <Show when={props.source}><span class="tine-rename-alias-tag">{t("rename-alias-dialog-this-page")}</span></Show>
    </li>
  );
}

function ownRefusal(offer: AliasDialogOffer, to: string): string | null {
  const refused = offer.ownRefused;
  if (!refused) return null;
  if (refused.kind === "plan") return render(renameRefusalMessage(refused.reason));
  if (refused.kind === "aliasLine") return t("rename-alias-dialog-alias-line", { to, page: refused.page });
  return t("rename-alias-dialog-other");
}

function Dialog(props: { request: AliasDialogRequest }) {
  const request = props.request;
  const offer = request.offer;
  const owner = offer.opens.name;
  const answer = (choice: Exclude<AliasDialogAnswer, null>) => {
    if (aliasDialogRequest() !== request) return refuseStaleWrite(t("rename-alias-dialog-stale-what"));
    request.finish(choice);
  };
  const sourceDeclares = offer.declarers.some((page) => page.path === offer.source.path && page.path !== null);
  const own = ownRefusal(offer, request.to);
  return (
    <RenameDialogFrame id="tine-rename-alias-dialog" label={t("rename-alias-dialog-title", { to: request.to, owner })} dismiss={() => request.finish(null)}>
        <h2>{t("rename-alias-dialog-title", { to: request.to, owner })}</h2>
        <p>{t("rename-alias-dialog-intro", { to: request.to, from: request.from })}</p>
        <ul class="tine-rename-alias-component">
          <For each={offer.declarers}>{(page) => <PageLine page={page} to={request.to} opens={page.path === offer.opens.path} source={page.path === offer.source.path} />}</For>
          <Show when={!sourceDeclares}><PageLine page={offer.source} to={request.to} opens={false} source /></Show>
        </ul>
        <div class="tine-rename-alias-choice" data-choice-block="merge">
          <Show when={offer.merge} fallback={
            <Show when={offer.mergeRefused}>{(reason) =>
              <p class="tine-rename-alias-unavailable">{t("rename-alias-dialog-merge-unavailable", { owner, reason: render(renameRefusalMessage(reason())) })}</p>}</Show>}>
            {(merge) => <>
              <h3>{t("rename-alias-dialog-merge", { from: request.from, owner: merge().name })}</h3>
              <ul>
                <li>{t("rename-alias-dialog-merge-body", { from: request.from, owner: merge().name })}</li>
                <Show when={merge().tags}>{(tags) => <li>{t("rename-alias-dialog-tags", { united: tags().united })}</li>}</Show>
                <Effect effect={merge().effect} />
              </ul>
            </>}
          </Show>
        </div>
        <div class="tine-rename-alias-choice" data-choice-block="own">
          <Show when={offer.own} fallback={<Show when={own}><p class="tine-rename-alias-unavailable">{t("rename-alias-dialog-own-unavailable", { to: request.to, reason: own ?? "" })}</p></Show>}>
            {(choice) => <>
              <h3>{t("rename-alias-dialog-own", { to: request.to })}</h3>
              <ul>
                <For each={choice().edits}>{(edit) => <li data-alias-edit={edit.page}>{edit.after === null
                  ? t("rename-alias-dialog-own-edit-line", { page: edit.page, key: edit.key, before: edit.before })
                  : t("rename-alias-dialog-own-edit", { to: request.to, page: edit.page, key: edit.key, before: edit.before, after: edit.after })}</li>}</For>
                <li>{t("rename-alias-dialog-own-body", { from: request.from, to: request.to })}</li>
                <Effect effect={choice().effect} />
              </ul>
            </>}
          </Show>
        </div>
        <Show when={!offer.merge && !offer.own}><p>{t("rename-alias-dialog-none")}</p></Show>
        <DialogActions
          aux={<Show when={offer.merge}>{(merge) =>
            <button type="button" data-choice="merge" onClick={() => answer("merge")}>{t("rename-alias-dialog-merge", { from: request.from, owner: merge().name })}</button>}</Show>}
          cancel={<button type="button" data-choice-cancel onClick={() => request.finish(null)}>{t("confirm-cancel")}</button>}
          primary={<Show when={offer.own}>
            <button type="button" data-choice="own" onClick={() => answer("own")}>{t("rename-alias-dialog-own", { to: request.to })}</button></Show>}
        />
    </RenameDialogFrame>
  );
}
