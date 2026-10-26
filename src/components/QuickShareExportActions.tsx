import { Show, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { blockProperty } from "../document";
import { SHARE_PROPERTY } from "../exportsPage";
import { msg, t } from "../i18n";
import { bindingOwner, graphOwner, ownedWhen, readOwned, writeOwned, type Owned } from "../owned";
import { pendingRecordId, removeShareProperty, finishSharePending, shareGistId } from "../quickShare/definitions";
import { QUICK_SHARE_OFF, quickShareErrorText, type QuickShareListing } from "../quickShare/commands";
import { readOr } from "../resourceRead";
import { pushToast } from "../toasts";
import { settingsOpen } from "../ui";

// Query results can mount many export definitions together. Share their in-flight
// local read, but never retain a listing across later actions/account changes.
let listingRead: Promise<Owned<QuickShareListing>> | undefined;
async function readListing(): Promise<QuickShareListing | undefined> {
  const answer = await (listingRead ??= readOwned(graphOwner(), backend().quickShareList()).finally(() => { listingRead = undefined; }));
  return answer.kind === "current" ? answer.value : undefined;
}

/** Actions on a share definition, including its occurrence in the Exports
 * overview. Keys and account authorization stay behind the existing commands;
 * graph cleanup uses the same guarded save as Settings → Sharing. */
export function QuickShareExportActions(props: { hostBlockId: string }): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  const [busy, setBusy] = createSignal(false);
  const [note, setNote] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const value = () => blockProperty(props.hostBlockId, SHARE_PROPERTY);
  const gistId = () => {
    const current = value();
    if (!current || pendingRecordId(current)) return null;
    const id = shareGistId(current);
    return /^[0-9a-f]{20,32}$/.test(id) ? id : null;
  };
  // Returning from Settings must see a newly connected/disconnected account or
  // a forgotten key without needing a page reload.
  const [listing] = createResource(() => !settingsOpen() && gistId(), readListing);
  const listed = () => readOr(listing, QUICK_SHARE_OFF, "export share account") ?? QUICK_SHARE_OFF;
  const row = () => listed().shares.find(share => share.gistId === gistId());
  const canDelete = () => listed().enabled && !!listed().account && !listed().error && (!row() || row()!.accountMatches);
  const remove = async () => {
    const gist = gistId();
    if (!gist || busy() || !canDelete()) return;
    const owner = bindingOwner();
    // Reserve the action before confirmation too: two clicks cannot open two
    // confirmations that later delete the same share independently.
    setBusy(true); setError(null); setNote(null);
    try {
      const confirmed = await readOwned(ownedWhen(owner, () => alive), backend().confirm(t("quick-share-settings-delete-confirm"), {
        okLabel: t("quick-share-settings-delete"), cancelLabel: t("confirm-cancel"),
      }));
      if (confirmed.kind !== "current" || !confirmed.value || gistId() !== gist) return;
      const known = row();
      const answer = await writeOwned(owner, backend().quickShareDelete(known ? { shareId: known.shareId } : { gistId: gist }));
      if (answer.kind !== "current") return;
      if (answer.value === "gone") {
        if (alive) setNote(t("quick-share-settings-gone"));
        else pushToast(msg("quick-share-settings-gone"), "info");
        return;
      }
      pushToast(msg("quick-share-settings-deleted"), "success");
      try {
        await removeShareProperty(gist, owner);
        if (known && owner()) await finishSharePending(known.shareId, null, owner);
      } catch (failure) {
        const args = { error: failure instanceof Error ? failure.message : String(failure) };
        if (alive) setError(t("quick-share-settings-unlisted-failed", args));
        // The result row may disappear as soon as its property is removed from
        // the working set, even when the disk save fails. Keep that failure visible.
        pushToast(msg("quick-share-settings-unlisted-failed", args), "error");
      }
    } catch (failure) {
      if (alive) setError(quickShareErrorText(failure));
      else pushToast(msg("quick-share-toast-failed", { error: quickShareErrorText(failure) }), "error");
    } finally {
      if (alive) setBusy(false);
    }
  };
  return <><Show when={gistId()}>
    <span class="quick-share-export-controls" onClick={event => event.stopPropagation()} onMouseDown={event => event.stopPropagation()}>
      <Show when={listed().enabled}>
        <button type="button" class="export-btn-secondary quick-share-export-delete" disabled={busy() || !canDelete()}
          onClick={() => void remove()}>{t("quick-share-settings-delete")}</button>
      </Show>
      <Show when={listed().error}><small role="alert">{listed().error}</small></Show>
      <Show when={row() && !row()!.accountMatches}><small>{t("quick-share-settings-other-account", { login: row()!.login })}</small></Show>
      <Show when={note()}><small role="status">{note()}</small></Show>
      <Show when={listing.error}><small role="alert">{quickShareErrorText(listing.error)}</small></Show>
    </span>
  </Show><Show when={error()}><small role="alert">{error()}</small></Show></>;
}
