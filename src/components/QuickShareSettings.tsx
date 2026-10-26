import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { msg, t } from "../i18n";
import { backend } from "../backend";
import { bindingOwner, graphOwner, latestOwner, ownedWhen, readOwned, writeOwned, type WriteOwner } from "../owned";
import { pushToast } from "../toasts";
import {
  GITHUB_APPLICATION_SETTINGS, quickShareErrorText,
  type QuickShareConnectionTest, type QuickShareFound, type QuickShareListed, type QuickShareListing,
} from "../quickShare/commands";
import { QuickShareConnect } from "./QuickShareDialog";
import { finishSharePending, pendingShareMarkers, removeShareProperty, shareValue, type PendingShareMarker } from "../quickShare/definitions";

/** Settings → Sharing (DESIGN §0, §3.6, §3.9, §8): the connected GitHub account, Find shares on GitHub, and the
 *  share keys this device holds, with Copy link, Check, Delete and Forget. What was shared is listed on the
 *  Exports page (`tine.share::` definitions); Delete and Forget remove that property too (D-20261018-Q1). Shares
 *  still marked `tine.share:: pending/<record id>` in the graph are listed with what Check can do for them
 *  (D-20261018-QS3): finish one recorded on this device, say "pending on another device" for another id, and offer
 *  to remove a marker no record here stands behind. Every GitHub action is native; the webview never holds a
 *  link, a key or the token. */
export function QuickShareSettings(): JSX.Element {
  const scope = {};
  let alive = true;
  onCleanup(() => { alive = false; });
  const [listing, setListing] = createSignal<QuickShareListing | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal<string | null>(null);
  const [connecting, setConnecting] = createSignal(false);
  const [connectionTest, setConnectionTest] = createSignal<QuickShareConnectionTest | null>(null);
  // Per-row results this session: "No longer on GitHub" (404 for the matching account) or "Not found yet".
  const [gone, setGone] = createSignal<ReadonlySet<string>>(new Set());
  const [notYet, setNotYet] = createSignal<ReadonlySet<string>>(new Set());
  const [found, setFound] = createSignal<QuickShareFound[] | null>(null);
  // Pending markers in the bound graph (D-20261018-QS3), and whether the query answering them was cut.
  const [markers, setMarkers] = createSignal<PendingShareMarker[]>([]);
  const [markersCut, setMarkersCut] = createSignal(false);

  const refresh = async () => {
    const owner = latestOwner(scope, "list", graphOwner(() => alive));
    try {
      const answer = await readOwned(owner, backend().quickShareList());
      if (answer.kind === "current") setListing(answer.value);
    } catch (failure) {
      if (owner()) setError(quickShareErrorText(failure));
    }
    await scanMarkers();
  };
  const scanMarkers = async () => {
    const owner = latestOwner(scope, "markers", graphOwner(() => alive));
    try {
      const answer = await pendingShareMarkers(owner);
      if (answer && owner()) { setMarkers(answer.markers); setMarkersCut(answer.exceeded); }
    } catch (failure) {
      if (owner()) setError(t("quick-share-settings-markers-failed", { error: failure instanceof Error ? failure.message : String(failure) }));
    }
  };
  onMount(() => void refresh());

  /** Run one action; its failure is shown in the tab and kept there (never a silent loss). Its outcome and graph
   *  bookkeeping belong to the graph the action started on (`owner`, captured before any confirmation), not to
   *  this view: closing Settings keeps them, a graph switch cancels them (R4 / I-20). */
  const act = async <T,>(key: string, owner: WriteOwner, work: () => Promise<T>, then: (value: T) => void | Promise<void>) => {
    if (busy() || !owner()) return;
    setBusy(key); setError(null);
    try {
      const answer = await writeOwned(owner, work());
      if (answer.kind === "current") await then(answer.value);
    } catch (failure) {
      if (alive) setError(quickShareErrorText(failure));
      else pushToast(msg("quick-share-toast-failed", { error: quickShareErrorText(failure) }), "error");
    } finally {
      if (alive) setBusy(null);
    }
  };
  const confirmed = async (message: string, action: "delete" | "forget" | "marker-remove", owner: WriteOwner) => {
    const answer = await readOwned(
      ownedWhen(() => alive, owner),
      backend().confirm(message, {
        okLabel: t(`quick-share-settings-${action}`),
        cancelLabel: t("confirm-cancel"),
      }),
    );
    return answer.kind === "current" && answer.value;
  };
  const add = (set: ReadonlySet<string>, id: string) => new Set([...set, id]);
  /** A share that is gone from this device's list is gone from the graph's listing too: every `tine.share::`
   *  naming its gist, whatever login prefix it carries (an account rename). Its failure is shown; the share
   *  itself is already deleted or forgotten. */
  const unlist = async (gistId: string | null, owner: WriteOwner, shareId?: string) => {
    if (!gistId && !shareId) return;
    try {
      if (gistId) await removeShareProperty(gistId, owner);
      // A leftover `pending/<record id>` of this share (its finish failed, or it was never confirmed).
      if (shareId) await finishSharePending(shareId, null, owner);
    } catch (failure) {
      const args = { error: failure instanceof Error ? failure.message : String(failure) };
      if (alive) setError(t("quick-share-settings-unlisted-failed", args)); else pushToast(msg("quick-share-settings-unlisted-failed", args), "error");
    }
  };

  const disconnect = () => void act("disconnect", bindingOwner(), () => backend().quickShareDisconnect(), () => { setFound(null); return refresh(); });
  const testConnection = () => {
    setConnectionTest(null);
    void act("connection-test", bindingOwner(), () => backend().quickShareTestConnection(), setConnectionTest);
  };
  const copy = (row: QuickShareListed) => void act(`copy:${row.shareId}`, bindingOwner(), () => backend().quickShareCopyLink(row.shareId),
    (copied) => { if (copied) pushToast(msg("quick-share-settings-copied"), "success"); else if (alive) setError(t("quick-share-copy-failed")); });
  /** A share recorded live here whose graph block still says pending: the marker becomes `<login>/<gist_id>`. */
  const finishMarker = async (shareId: string, owner: WriteOwner) => {
    const row = listing()?.shares.find((share) => share.shareId === shareId);
    if (!row || row.state !== "live" || !row.gistId || !markers().some((marker) => marker.shareId === shareId)) return;
    try {
      const pages = await finishSharePending(shareId, shareValue(row.login, row.gistId), owner);
      if (pages.length) pushToast(msg("quick-share-definition-saved", { page: pages.join(", ") }), "success");
    } catch (failure) {
      const args = { error: failure instanceof Error ? failure.message : String(failure) };
      if (alive) setError(t("quick-share-definition-failed", args)); else pushToast(msg("quick-share-definition-failed", args), "error");
    }
  };
  const check = (row: QuickShareListed) => {
    const owner = bindingOwner();
    void act(`check:${row.shareId}`, owner, () => backend().quickShareCheck(row.shareId), async (outcome) => {
      if (outcome !== "live") { setNotYet((set) => add(set, row.shareId)); return; }
      await refresh();
      await finishMarker(row.shareId, owner);
      await scanMarkers();
    });
  };
  /** Check every pending marker in this graph (D-20261018-QS3): a share recorded here and still pending is looked
   *  for on GitHub first (a gist created but never acknowledged); one recorded live gets its marker finished.
   *  Markers of other devices, or of other accounts, are left as they are. */
  const checkMarkers = async () => {
    const owner = bindingOwner();
    if (busy() || !owner()) return;
    setBusy("markers"); setError(null);
    try {
      for (const marker of markers()) {
        if (!owner()) return;
        const row = listing()?.shares.find((share) => share.shareId === marker.shareId);
        if (!row) continue;
        if (row.state === "pending") {
          if (!row.accountMatches) continue;
          const answer = await writeOwned(owner, backend().quickShareCheck(row.shareId));
          if (answer.kind === "stale") return;
          if (answer.value !== "live") { setNotYet((set) => add(set, row.shareId)); continue; }
          await refresh();
        }
        await finishMarker(row.shareId, owner);
      }
      await scanMarkers();
    } catch (failure) {
      if (alive) setError(quickShareErrorText(failure));
      else pushToast(msg("quick-share-toast-failed", { error: quickShareErrorText(failure) }), "error");
    } finally {
      if (alive) setBusy(null);
    }
  };
  /** Remove a marker no record on this device stands behind (another device's share in flight, or one whose
   *  record is gone), after the user confirmed. */
  const removeMarker = async (marker: PendingShareMarker) => {
    const owner = bindingOwner();
    if (busy() || !(await confirmed(t("quick-share-settings-marker-remove-confirm"), "marker-remove", owner))) return;
    setBusy(`marker:${marker.shareId}`); setError(null);
    try {
      await finishSharePending(marker.shareId, null, owner);
      await scanMarkers();
    } catch (failure) {
      const args = { error: failure instanceof Error ? failure.message : String(failure) };
      if (alive) setError(t("quick-share-pending-unmark-failed", args)); else pushToast(msg("quick-share-pending-unmark-failed", args), "error");
    } finally {
      if (alive) setBusy(null);
    }
  };
  const remove = async (row: QuickShareListed) => {
    const owner = bindingOwner();
    if (busy() || !(await confirmed(t("quick-share-settings-delete-confirm"), "delete", owner))) return;
    void act(`delete:${row.shareId}`, owner, () => backend().quickShareDelete({ shareId: row.shareId }), async (outcome) => {
      if (outcome === "gone") { setGone((set) => add(set, row.shareId)); return; }
      pushToast(msg("quick-share-settings-deleted"), "success");
      await unlist(row.gistId, owner, row.shareId);
      return refresh();
    });
  };
  const forget = async (row: QuickShareListed) => {
    const owner = bindingOwner();
    if (busy() || !(await confirmed(t("quick-share-settings-forget-confirm"), "forget", owner))) return;
    void act(`forget:${row.shareId}`, owner, () => backend().quickShareForget(row.shareId), async () => {
      await unlist(row.gistId, owner, row.shareId);
      return refresh();
    });
  };
  const find = async () => {
    if (busy()) return;
    setBusy("find"); setError(null); setFound(null);
    const owner = latestOwner(scope, "find", graphOwner(() => alive));
    try {
      const answer = await readOwned(owner, backend().quickShareFind());
      if (answer.kind === "current") setFound(answer.value);
    } catch (failure) {
      if (owner()) setError(quickShareErrorText(failure));
    } finally {
      if (alive) setBusy(null);
    }
  };
  const removeFound = async (gist: QuickShareFound) => {
    const owner = bindingOwner();
    if (busy() || !(await confirmed(t("quick-share-settings-delete-confirm"), "delete", owner))) return;
    void act(`delete:${gist.gistId}`, owner, () => backend().quickShareDelete({ gistId: gist.gistId }), async () => {
      setFound((list) => list?.filter((item) => item.gistId !== gist.gistId) ?? null);
      pushToast(msg("quick-share-settings-deleted"), "success");
      // By gist identity: a reference written before an account rename carries the old login.
      await unlist(gist.gistId, owner);
    });
  };
  const openApplications = () => void readOwned(graphOwner(), backend().openExternal(GITHUB_APPLICATION_SETTINGS))
    .catch((failure) => setError(quickShareErrorText(failure)));
  const date = (iso: string) => { const parsed = new Date(iso); return Number.isNaN(parsed.getTime()) ? iso : parsed.toLocaleDateString(); };

  return (
    <div class="quick-share-settings">
      <Show when={listing()?.connectionTestAvailable}>
        <button type="button" class="export-btn-secondary quick-share-test-connection" disabled={!!busy()} onClick={testConnection}>{t("quick-share-connection-test")}</button>
        <Show when={connectionTest()}>{(result) => <p role="status" class="settings-hint">{t(`quick-share-connection-${result()}`)}</p>}</Show>
      </Show>
      <div class="settings-section">{t("quick-share-settings-account")}</div>
      <Show when={listing()?.account} fallback={
        <Show when={connecting()} fallback={<>
          <p class="settings-hint">{t("quick-share-settings-not-connected")}</p>
          <button type="button" class="export-btn-primary quick-share-settings-connect" onClick={() => setConnecting(true)}>{t("quick-share-connect-button")}</button>
        </>}>
          <QuickShareConnect onConnected={() => { setConnecting(false); void refresh(); }} />
        </Show>
      }>{(account) => <>
        <p class="quick-share-account">{t("quick-share-connected-as", { login: account().login })}</p>
        <div class="quick-share-settings-actions">
          <button type="button" class="export-btn-secondary quick-share-disconnect" disabled={!!busy()} onClick={disconnect}>{t("quick-share-settings-disconnect")}</button>
          <button type="button" class="export-btn-secondary quick-share-revoke" onClick={openApplications}>{t("quick-share-settings-revoke-link")}</button>
        </div>
        <p class="settings-hint">{t("quick-share-settings-revoke")}</p>
      </>}</Show>

      <div class="settings-section">{t("quick-share-settings-shares")}</div>
      <p class="settings-hint quick-share-settings-shares-hint">{t("quick-share-settings-shares-hint")}</p>
      <Show when={listing()?.error}><div role="alert" class="query-export-refused">{listing()!.error}</div></Show>
      <Show when={listing() && !listing()!.error && listing()!.shares.length === 0}>
        <p class="settings-hint quick-share-settings-none">{t("quick-share-settings-none")}</p>
      </Show>
      <ul class="quick-share-shares">
        <For each={listing()?.shares ?? []}>{(row) => {
          const isGone = () => gone().has(row.shareId);
          return <li class="quick-share-share" data-share-id={row.shareId} data-state={isGone() ? "gone" : row.state}>
            <div class="quick-share-share-title">{row.title}</div>
            <small class="settings-hint">{t("quick-share-settings-row", { pages: row.pages, date: date(row.createdAt), login: row.login })}</small>
            <Show when={isGone()}><small class="quick-share-share-state">{t("quick-share-settings-gone")}</small></Show>
            <Show when={!isGone() && row.state === "pending"}>
              <small class="quick-share-share-state">{notYet().has(row.shareId) ? t("quick-share-check-not-yet") : t("quick-share-settings-pending")}</small>
            </Show>
            <Show when={!row.accountMatches}><small class="settings-hint">{t("quick-share-settings-other-account", { login: row.login })}</small></Show>
            <div class="quick-share-settings-actions">
              <Show when={!isGone() && row.state === "live"}>
                <button type="button" class="export-btn-secondary quick-share-row-copy" disabled={!!busy()} onClick={() => copy(row)}>{t("quick-share-settings-copy")}</button>
              </Show>
              <Show when={!isGone() && row.state === "pending" && row.accountMatches}>
                <button type="button" class="export-btn-secondary quick-share-row-check" disabled={!!busy()} onClick={() => check(row)}>{t("quick-share-check")}</button>
              </Show>
              <Show when={!isGone() && row.state === "live" && row.accountMatches}>
                <button type="button" class="export-btn-secondary quick-share-row-delete" disabled={!!busy()} onClick={() => void remove(row)}>{t("quick-share-settings-delete")}</button>
              </Show>
              <Show when={isGone() || row.state === "pending"}>
                <button type="button" class="export-btn-secondary quick-share-row-forget" disabled={!!busy()} onClick={() => void forget(row)}>{t("quick-share-settings-forget")}</button>
              </Show>
            </div>
          </li>;
        }}</For>
      </ul>

      <Show when={markers().length > 0}>
        <div class="settings-section">{t("quick-share-settings-markers")}</div>
        <p class="settings-hint">{t("quick-share-settings-markers-hint")}</p>
        <ul class="quick-share-markers"><For each={markers()}>{(marker) => {
          const row = () => listing()?.shares.find((share) => share.shareId === marker.shareId);
          const state = () => {
            const known = row();
            if (!known) return t("quick-share-settings-marker-elsewhere");
            if (known.state === "live") return t("quick-share-settings-marker-live");
            if (!known.accountMatches) return t("quick-share-settings-other-account", { login: known.login });
            return notYet().has(known.shareId) ? t("quick-share-check-not-yet") : t("quick-share-settings-pending");
          };
          return <li class="quick-share-marker" data-share-id={marker.shareId} data-known={row() ? "yes" : "no"}>
            <div class="quick-share-share-title">{row()?.title ?? t("quick-share-settings-marker-unknown")}</div>
            <small class="settings-hint">{t("quick-share-settings-marker-pages", { pages: marker.pages.join(", ") })}</small>
            <small class="quick-share-share-state">{state()}</small>
            <Show when={!row()}>
              <div class="quick-share-settings-actions">
                <button type="button" class="export-btn-secondary quick-share-marker-remove" disabled={!!busy()} onClick={() => void removeMarker(marker)}>{t("quick-share-settings-marker-remove")}</button>
              </div>
            </Show>
          </li>;
        }}</For></ul>
        <Show when={markersCut()}><p class="settings-hint">{t("quick-share-settings-markers-cut")}</p></Show>
        <Show when={markers().some((marker) => listing()?.shares.some((share) => share.shareId === marker.shareId))}>
          <button type="button" class="export-btn-secondary quick-share-markers-check" disabled={!!busy()} onClick={() => void checkMarkers()}>
            {busy() === "markers" ? t("quick-share-settings-markers-checking") : t("quick-share-check")}
          </button>
        </Show>
      </Show>

      <Show when={listing()?.account}>
        <div class="settings-section">{t("quick-share-settings-find")}</div>
        <p class="settings-hint">{t("quick-share-settings-find-hint")}</p>
        <button type="button" class="export-btn-secondary quick-share-find" disabled={!!busy()} onClick={() => void find()}>
          {busy() === "find" ? t("quick-share-settings-finding") : t("quick-share-settings-find")}
        </button>
        <Show when={found()}>{(list) => <>
          <Show when={list().length === 0}><p class="settings-hint quick-share-found-none">{t("quick-share-settings-found-none")}</p></Show>
          <ul class="quick-share-found"><For each={list()}>{(gist) =>
            <li class="quick-share-found-row" data-gist-id={gist.gistId}>
              <div>{t("quick-share-settings-found-row")}</div>
              <small class="settings-hint">{date(gist.createdAt)}</small>
              <button type="button" class="export-btn-secondary quick-share-found-delete" disabled={!!busy()} onClick={() => void removeFound(gist)}>{t("quick-share-settings-delete")}</button>
            </li>
          }</For></ul>
        </>}</Show>
      </Show>
      <Show when={error()}><div role="alert" class="query-export-refused quick-share-error">{error()}</div></Show>
    </div>
  );
}
