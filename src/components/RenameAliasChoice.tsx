import { t } from "../i18n";
import { onCleanup, onMount, Show } from "solid-js";
import { registerTransientLayer } from "../transientLayers";
import { refuseStaleWrite } from "../binding";
import { documentOf } from "../windowRealm";
import { aliasRenameRequest, type AliasRenameRequest } from "../renameAliasChoice";
import { DialogActions } from "./DialogActions";
import "./RenameAliasChoice.css";

/** The M-Q5 rename dialog (D-20261102-MC5): the new name is only another
 *  page's alias. Merge into that page / make the name its own page / Cancel.
 *  The backend decided the question (`rename_offer`); this only asks it. */
export function RenameAliasChoice() {
  return <Show when={aliasRenameRequest()}>{(request) => <Choice request={request()} />}</Show>;
}

function Choice(props: { request: AliasRenameRequest }) {
  const request = props.request;
  let root!: HTMLElement;
  onCleanup(() => request.finish(null));
  // A graph switch has already cancelled the request; an answer that still
  // arrives renames nothing (the rename itself also checks its owner).
  const answer = (choice: "merge" | "own-page") => {
    if (aliasRenameRequest() !== request) return refuseStaleWrite(t("rename-alias-choice-stale-what"));
    request.finish(choice);
  };
  onMount(() => {
    onCleanup(registerTransientLayer({ id: "tine-rename-alias-choice", root: () => root,
      dismiss: () => { request.finish(null); return true; } }));
    root.querySelector<HTMLButtonElement>(request.ownPage ? 'button[data-choice="own-page"]' : 'button[data-choice="merge"]')?.focus();
  });
  return (
    <div class="modal-overlay tine-rename-alias-overlay" onKeyDown={(event) => {
      if (event.key === "Escape") request.finish(null);
      if (event.key === "Tab") {
        const buttons = [...root.querySelectorAll<HTMLButtonElement>("button")];
        const first = buttons[0], last = buttons.at(-1);
        if (event.shiftKey && documentOf(event).activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && documentOf(event).activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    }}>
      <section ref={root} class="tine-rename-alias-choice" role="dialog" aria-modal="true" aria-label={t("rename-alias-choice-title", { to: request.to })}>
        <h2>{t("rename-alias-choice-title", { to: request.to })}</h2>
        <p>{t(request.ownPage ? "rename-alias-choice-body" : "rename-alias-choice-fileless", { to: request.to, owner: request.owner, from: request.from })}</p>
        <Show when={!request.merge}>
          <p>{t("rename-alias-choice-no-merge", { owner: request.owner, from: request.from })}</p>
        </Show>
        {/* The merge trashes this page into the owner, so it is the auxiliary
            action (D-20261018-UI1); "own page" changes no other page. */}
        <DialogActions
          aux={<Show when={request.merge}>
            <button type="button" data-choice="merge" onClick={() => answer("merge")}>{t("rename-alias-choice-merge", { owner: request.owner })}</button>
          </Show>}
          cancel={<button type="button" data-choice="cancel" onClick={() => request.finish(null)}>{t("confirm-cancel")}</button>}
          primary={<Show when={request.ownPage}><button type="button" data-choice="own-page" onClick={() => answer("own-page")}>{t("rename-alias-choice-own-page", { to: request.to, owner: request.owner })}</button></Show>}
        />
      </section>
    </div>
  );
}
