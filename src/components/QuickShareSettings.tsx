import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { msg, t } from "../i18n";
import { backend } from "../backend";
import { bindingOwner, graphOwner, latestOwner, readOwned, writeOwned } from "../owned";
import { pushToast } from "../toasts";
import {
  GITHUB_APPLICATION_SETTINGS, quickShareErrorText,
  type QuickShareFound, type QuickShareListed, type QuickShareListing,
} from "../quickShare/commands";
import { QuickShareConnect } from "./QuickShareDialog";
import { removeShareProperty, shareValue } from "../quickShare/definitions";

/** Settings → Sharing (DESIGN §0, §3.6, §3.9, §8): the connected GitHub account, Find shares on GitHub, and the
 *  share keys this device holds, with Copy link, Check, Delete and Forget. What was shared is listed on the
 *  Exports page (`tine.share::` definitions); Delete and Forget remove that property too (D-20261018-Q1). Every
 *  GitHub action is native; the webview never holds a link, a key or the token. */
export function QuickShareSettings(): JSX.Element {
  const scope = {};
  let alive = true;
  onCleanup(() => { alive = false; });
  const [listing, setListing] = createSignal<QuickShareListing | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal<string | null>(null);
  const [connecting, setConnecting] = createSignal(false);
  // Per-row results this session: "No longer on GitHub" (404 for the matching account) or "Not found yet".
  const [gone, setGone] = createSignal<ReadonlySet<string>>(new Set());
  const [notYet, setNotYet] = createSignal<ReadonlySet<string>>(new Set());
  const [found, setFound] = createSignal<QuickShareFound[] | null>(null);

  const refresh = async () => {
    const owner = latestOwner(scope, "list", graphOwner(() => alive));
    try {
      const answer = await readOwned(owner, backend().quickShareList());
      if (answer.kind === "current") setListing(answer.value);
    } catch (failure) {
      if (owner()) setError(quickShareErrorText(failure));
    }
  };
  onMount(() => void refresh());

  /** Run one action; its failure is shown in the tab and kept there (never a silent loss). */
  const act = async <T,>(key: string, work: () => Promise<T>, then: (value: T) => void | Promise<void>) => {
    if (busy()) return;
    setBusy(key); setError(null);
    try {
      const answer = await writeOwned(bindingOwner(() => alive), work());
      if (answer.kind === "current") await then(answer.value);
    } catch (failure) {
      if (alive) setError(quickShareErrorText(failure));
      else pushToast(msg("quick-share-toast-failed", { error: quickShareErrorText(failure) }), "error");
    } finally {
      if (alive) setBusy(null);
    }
  };
  const confirmed = async (message: string) => {
    const answer = await readOwned(bindingOwner(() => alive), backend().confirm(message));
    return answer.kind === "current" && answer.value;
  };
  const add = (set: ReadonlySet<string>, id: string) => new Set([...set, id]);
  /** A share that is gone from this device's list is gone from the graph's listing too. Its failure is shown;
   *  the share itself is already deleted or forgotten. */
  const unlist = async (login: string, gistId: string | null) => {
    if (!gistId) return;
    try {
      await removeShareProperty(shareValue(login, gistId));
    } catch (failure) {
      const args = { error: failure instanceof Error ? failure.message : String(failure) };
      if (alive) setError(t("quick-share-settings-unlisted-failed", args)); else pushToast(msg("quick-share-settings-unlisted-failed", args), "error");
    }
  };

  const disconnect = () => void act("disconnect", () => backend().quickShareDisconnect(), () => { setFound(null); return refresh(); });
  const copy = (row: QuickShareListed) => void act(`copy:${row.shareId}`, () => backend().quickShareCopyLink(row.shareId),
    (copied) => { if (copied) pushToast(msg("quick-share-settings-copied"), "success"); else setError(t("quick-share-copy-failed")); });
  const check = (row: QuickShareListed) => void act(`check:${row.shareId}`, () => backend().quickShareCheck(row.shareId), (outcome) => {
    if (outcome === "live") return refresh();
    setNotYet((set) => add(set, row.shareId));
  });
  const remove = async (row: QuickShareListed) => {
    if (busy() || !(await confirmed(t("quick-share-settings-delete-confirm")))) return;
    void act(`delete:${row.shareId}`, () => backend().quickShareDelete({ shareId: row.shareId }), async (outcome) => {
      if (outcome === "gone") { setGone((set) => add(set, row.shareId)); return; }
      pushToast(msg("quick-share-settings-deleted"), "success");
      await unlist(row.login, row.gistId);
      return refresh();
    });
  };
  const forget = async (row: QuickShareListed) => {
    if (busy() || !(await confirmed(t("quick-share-settings-forget-confirm")))) return;
    void act(`forget:${row.shareId}`, () => backend().quickShareForget(row.shareId), async () => {
      await unlist(row.login, row.gistId);
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
    if (busy() || !(await confirmed(t("quick-share-settings-delete-confirm")))) return;
    void act(`delete:${gist.gistId}`, () => backend().quickShareDelete({ gistId: gist.gistId }), async () => {
      setFound((list) => list?.filter((item) => item.gistId !== gist.gistId) ?? null);
      pushToast(msg("quick-share-settings-deleted"), "success");
      // Find lists only the connected account's gists, so that account's login names it in the graph.
      const login = listing()?.account?.login;
      if (login) await unlist(login, gist.gistId);
    });
  };
  const openApplications = () => void readOwned(graphOwner(), backend().openExternal(GITHUB_APPLICATION_SETTINGS))
    .catch((failure) => setError(quickShareErrorText(failure)));
  const date = (iso: string) => { const parsed = new Date(iso); return Number.isNaN(parsed.getTime()) ? iso : parsed.toLocaleDateString(); };

  return (
    <div class="quick-share-settings">
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
