import { For, Show, createSignal, onCleanup, onMount, type Accessor, type JSX } from "solid-js";
import { DialogActions } from "./DialogActions";
import { msg, t } from "../i18n";
import { backend } from "../backend";
import { bindingOwner, graphOwner, latestOwner, readOwned, readOwnedResource, writeOwned, type WriteOwner } from "../owned";
import { dbg } from "../debug";
import { pushToast } from "../toasts";
import { closeQuickShare, quickShareDefinition } from "../shareState";
import { GRAPH_CHANGED, finishSharePending, markSharePending, shareValue } from "../quickShare/definitions";
import { registerTransientLayer } from "../transientLayers";
import type { QueryPublicationRequest } from "../types";
import {
  quickShareConnectText, quickShareErrorText, quickShareSize,
  type QuickShareConnectStart, type QuickShareConnectStatus, type QuickShareCreated, type QuickShareError,
  type QuickShareListing, type QuickSharePrepared,
} from "../quickShare/commands";
import { trapTab } from "./focusableElements";

/** How often the connect screen asks the native device flow for its state (a local call; Rust polls GitHub). */
export const QUICK_SHARE_STATUS_POLL_MS = 1000;
const LAYER_ID = "quick-share-dialog";

const kindOf = (error: unknown): QuickShareError["kind"] | null =>
  typeof error === "object" && error !== null && typeof (error as { kind?: unknown }).kind === "string"
    ? (error as QuickShareError).kind : null;

/** Connect GitHub (DESIGN §0, §3.4): shows the code, opens github.com/login/device, and follows the native flow
 *  until it ends. Leaving it cancels a pending attempt. Used by the quick share dialog and Settings → Sharing. */
export function QuickShareConnect(props: { onConnected: (login: string) => void }): JSX.Element {
  const [attempt, setAttempt] = createSignal<QuickShareConnectStart | null>(null);
  const [status, setStatus] = createSignal<QuickShareConnectStatus | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [starting, setStarting] = createSignal(false);
  let alive = true;
  const cancel = (id: string) => {
    void readOwned(graphOwner(), backend().quickShareConnectCancel(id))
      .catch((failure) => dbg(`quick share: cancelling a connection failed: ${String(failure)}`));
  };
  onCleanup(() => {
    alive = false;
    const current = attempt();
    if (current && status()?.state === "pending") cancel(current.attemptId);
  });
  const follow = async (id: string) => {
    const owner = graphOwner(() => alive && attempt()?.attemptId === id);
    while (owner()) {
      await new Promise((resolve) => setTimeout(resolve, QUICK_SHARE_STATUS_POLL_MS));
      if (!owner()) return;
      try {
        const answer = await readOwned(owner, backend().quickShareConnectStatus(id));
        if (answer.kind === "stale") return;
        setStatus(answer.value);
        if (answer.value.state === "connected") { props.onConnected(answer.value.login); return; }
        if (answer.value.state !== "pending") return;
      } catch (failure) {
        if (owner()) setError(quickShareErrorText(failure));
        return;
      }
    }
  };
  const start = async () => {
    const previous = attempt();
    if (previous && status()?.state === "pending") cancel(previous.attemptId);
    setAttempt(null); setStatus(null); setError(null); setStarting(true);
    try {
      // A start that lands after the view closed has already set Rust polling GitHub: cancel it (DESIGN §3.4).
      const started = await readOwnedResource(bindingOwner(() => alive), backend().quickShareConnectStart(),
        (late) => cancel(late.attemptId));
      if (started.kind === "stale") return;
      setAttempt(started.value);
      setStatus({ state: "pending" });
      void readOwned(graphOwner(), backend().openExternal(started.value.verificationUri))
        .catch((failure) => dbg(`quick share: opening GitHub failed: ${String(failure)}`));
      void follow(started.value.attemptId);
    } catch (failure) {
      if (alive) setError(quickShareErrorText(failure));
      else pushToast(msg("quick-share-toast-failed", { error: quickShareErrorText(failure) }), "error");
    } finally {
      if (alive) setStarting(false);
    }
  };
  const ended = () => { const state = status()?.state; return !!state && state !== "pending" && state !== "connected"; };
  return (
    <div class="quick-share-connect">
      <p class="settings-hint">{t("quick-share-connect-permission")}</p>
      <p class="settings-hint">{t("quick-share-connect-google-tip")}</p>
      <Show when={attempt() && status()?.state === "pending"} fallback={
        <button type="button" class="export-btn-primary quick-share-connect-start" disabled={starting()} onClick={() => void start()}>
          {ended() ? t("quick-share-connect-again") : t("quick-share-connect-button")}
        </button>
      }>
        <div class="quick-share-code-label">{t("quick-share-connect-code")}</div>
        <div class="quick-share-code" aria-live="polite">{attempt()!.userCode}</div>
        <DialogActions
          cancel={<button type="button" class="export-btn-secondary quick-share-connect-cancel"
            onClick={() => { cancel(attempt()!.attemptId); setAttempt(null); setStatus(null); }}>{t("quick-share-cancel")}</button>}
          primary={<button type="button" class="export-btn-secondary quick-share-open-github"
            onClick={() => void readOwned(graphOwner(), backend().openExternal(attempt()!.verificationUri))
              .catch((failure) => setError(quickShareErrorText(failure)))}>{t("quick-share-connect-open")}</button>} />
      </Show>
      <Show when={status()}>{(current) => <p class="settings-hint quick-share-connect-status" role="status">{quickShareConnectText(current())}</p>}</Show>
      <Show when={error()}><div role="alert" class="query-export-refused">{error()}</div></Show>
    </div>
  );
}

/** The quick share dialog (DESIGN §3.2, §3.5, §3.9): connect if needed, prepare the reviewed request, show exactly
 *  what goes out, and create the link once. Errors name the cause and keep the dialog open. */
export function QuickShareDialog(props: { request: Accessor<QueryPublicationRequest | null> }): JSX.Element {
  return <Show when={props.request()}>{(request) => <Dialog request={request()} />}</Show>;
}

function Dialog(props: { request: QueryPublicationRequest }): JSX.Element {
  const scope = {};
  const [listing, setListing] = createSignal<QuickShareListing | null>(null);
  const [preview, setPreview] = createSignal<QuickSharePrepared["preview"] | null>(null);
  // The prepared payload that Create uploads; cleared the moment Create consumes it. `shareId` is the record id
  // that upload will use, which the query block carries as `tine.share:: pending/<shareId>` from the start.
  const [prepared, setPrepared] = createSignal<{ prepareId: string; shareId: string } | null>(null);
  const prepareId = () => prepared()?.prepareId ?? null;
  const [busy, setBusy] = createSignal<"list" | "prepare" | "create" | "check" | "copy" | null>("list");
  const [error, setError] = createSignal<string | null>(null);
  const [needsConnect, setNeedsConnect] = createSignal(false);
  const [created, setCreated] = createSignal<QuickShareCreated | null>(null);
  const [copied, setCopied] = createSignal(false);
  // A create whose outcome is unknown: the row stays pending and Check looks for it (DESIGN §3.5–3.6).
  const [pending, setPending] = createSignal<string | null>(null);
  const [checkNote, setCheckNote] = createSignal<string | null>(null);
  const [live, setLive] = createSignal<string | null>(null);
  // The page the share's `tine.share::` definition landed on (D-20261018-Q1).
  const [recorded, setRecorded] = createSignal<string | null>(null);
  let alive = true;

  /** Finish the share's pending marker in the graph (D-20261018-QS3): `<login>/<gist_id>` once the gist exists,
   *  removed (`null`) when it was not created. Found by the property wherever the block is now. The share stands
   *  either way, so this runs even when the dialog was closed meanwhile (the outcome is then a toast), but only in
   *  the graph the action started on: `owner` is that graph binding, never the dialog's lifetime. `markedOn` is
   *  the page the marker was saved to at the start. */
  const finish = async (shareId: string, next: string | null, owner: WriteOwner, markedOn: string | null) => {
    if (!markedOn) return;
    try {
      const pages = await finishSharePending(shareId, next, owner, [markedOn]);
      if (!owner() || next === null) return;
      const [page] = pages;
      if (page) { if (alive) setRecorded(page); else pushToast(msg("quick-share-definition-saved", { page }), "success"); }
      else if (alive) setError(t("quick-share-definition-missing"));
      else pushToast(msg("quick-share-definition-missing"), "error");
    } catch (failure) {
      const args = { error: failure instanceof Error ? failure.message : String(failure) };
      const key = next === null ? "quick-share-pending-unmark-failed" : "quick-share-definition-failed";
      if (alive) setError(t(key, args)); else pushToast(msg(key, args), "error");
    }
  };
  // The page the current share's pending marker was saved to (null: no definition, so no marker).
  let markedOn: string | null = null;

  const discard = (id: string) => {
    void readOwned(graphOwner(), backend().quickShareDiscard(id))
      .catch((failure) => dbg(`quick share: discarding a preview failed: ${String(failure)}`));
  };
  onCleanup(() => {
    alive = false;
    const id = prepareId();
    if (id) discard(id);
  });

  const prepare = async () => {
    const previous = prepareId();
    if (previous) discard(previous);
    setPrepared(null); setError(null); setBusy("prepare");
    const owner = latestOwner(scope, "prepare", graphOwner(() => alive));
    try {
      // A preview nobody shows any more is discarded at once, not left to expire.
      const answer = await readOwnedResource(owner, backend().quickSharePrepare(props.request), (stale) => discard(stale.prepareId));
      if (answer.kind === "stale") return;
      setPreview(answer.value.preview);
      setPrepared({ prepareId: answer.value.prepareId, shareId: answer.value.shareId });
    } catch (failure) {
      if (owner()) setError(quickShareErrorText(failure));
    } finally {
      if (owner()) setBusy(null);
    }
  };

  const load = async () => {
    const owner = graphOwner(() => alive);
    try {
      const answer = await readOwned(owner, backend().quickShareList());
      if (answer.kind === "stale") return;
      setListing(answer.value);
      setBusy(null);
      if (!answer.value.enabled) { setError(quickShareErrorText({ kind: "disabled" })); return; }
      if (!answer.value.account) { setNeedsConnect(true); return; }
      await prepare();
    } catch (failure) {
      if (owner()) { setBusy(null); setError(quickShareErrorText(failure)); }
    }
  };
  onMount(() => void load());

  // Connected: read the account back from the records, then prepare (again, after a reconnect).
  const connected = () => { setNeedsConnect(false); setError(null); void load(); };

  const create = async () => {
    const current = prepared();
    if (!current || busy()) return;
    setError(null); setBusy("create");
    // The graph bookkeeping belongs to the graph Create started on, not to this view: closing the dialog keeps
    // it, a graph switch cancels it (R4 / I-20).
    const owner = bindingOwner();
    const definition = quickShareDefinition(props.request);
    const { prepareId: id, shareId } = current;
    markedOn = null;
    if (definition) {
      // The marker is saved before anything is uploaded (D-20261018-QS3): without it a share confirmed later could
      // not find its block. If it cannot be saved, nothing is uploaded and the preview stays for another try.
      try {
        markedOn = await markSharePending(definition, shareId, owner);
      } catch (failure) {
        const reason = failure instanceof Error ? failure.message : String(failure);
        if (reason !== GRAPH_CHANGED && alive) setError(t("quick-share-pending-failed", { error: reason }));
        if (alive) setBusy(null);
        return;
      }
      if (!alive) {
        // Closed while the marker was saved: the share is cancelled before it started (the dialog's cleanup
        // discarded the payload), so the marker goes again.
        await finish(shareId, null, owner, markedOn);
        return;
      }
    }
    // Create consumes the payload whatever happens: a retry needs a fresh preview (never an automatic re-upload).
    setPrepared(null);
    try {
      const answer = await writeOwned(owner, backend().quickShareCreate(id));
      if (answer.kind === "stale") return;
      if (alive) {
        setCreated(answer.value);
        setCopied(answer.value.copied);
        if (!answer.value.copied) setError(t("quick-share-copy-failed"));
      }
      await finish(shareId, shareValue(answer.value.login, answer.value.gistId), owner, markedOn);
    } catch (failure) {
      const kind = kindOf(failure);
      // An unknown outcome keeps the marker (a later Check finishes it); any other failure made no gist.
      if (kind !== "maybeUploaded") await finish(shareId, null, owner, markedOn);
      if (!alive) { pushToast(msg("quick-share-toast-failed", { error: quickShareErrorText(failure) }), "error"); return; }
      if (kind === "maybeUploaded") setPending((failure as { shareId: string }).shareId);
      if (kind === "notConnected" || kind === "reconnect") setNeedsConnect(true);
      setError(quickShareErrorText(failure));
    } finally {
      if (alive) setBusy(null);
    }
  };

  const check = async () => {
    const shareId = pending();
    if (!shareId || busy()) return;
    setError(null); setCheckNote(null); setBusy("check");
    const owner = bindingOwner();
    try {
      const answer = await writeOwned(owner, backend().quickShareCheck(shareId));
      if (answer.kind === "stale") return;
      if (answer.value === "live") {
        if (alive) { setPending(null); setLive(shareId); setCheckNote(t("quick-share-check-found")); }
        // Check answers only the state; the gist it found is in the records.
        const listed = await readOwned(owner, backend().quickShareList());
        const row = listed.kind === "current" ? listed.value.shares.find((share) => share.shareId === shareId) : undefined;
        if (row?.gistId) await finish(shareId, shareValue(row.login, row.gistId), owner, markedOn);
      } else if (alive) setCheckNote(t("quick-share-check-not-yet"));
    } catch (failure) {
      if (alive) setError(quickShareErrorText(failure));
      else pushToast(msg("quick-share-toast-failed", { error: quickShareErrorText(failure) }), "error");
    } finally {
      if (alive) setBusy(null);
    }
  };

  const copyLink = async (shareId: string) => {
    if (busy()) return;
    setError(null); setBusy("copy");
    try {
      const answer = await writeOwned(bindingOwner(() => alive), backend().quickShareCopyLink(shareId));
      if (answer.kind === "stale") return;
      setCopied(answer.value);
      if (!answer.value) setError(t("quick-share-copy-failed"));
    } catch (failure) {
      if (alive) setError(quickShareErrorText(failure));
    } finally {
      if (alive) setBusy(null);
    }
  };

  let dialog!: HTMLDivElement;
  let overlay!: HTMLDivElement;
  onMount(() => {
    onCleanup(registerTransientLayer({ id: LAYER_ID, root: () => overlay ?? null, dismiss: () => { closeQuickShare(); return true; } }));
    queueMicrotask(() => dialog?.focus());
  });

  const done = () => !!created() || !!live();
  return (
    <div class="modal-overlay" ref={overlay} onClick={closeQuickShare}>
      <div ref={dialog} class="settings-modal share-dialog quick-share-dialog" role="dialog" aria-modal="true"
        aria-labelledby="quick-share-title" tabIndex={-1} onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => trapTab(event, dialog)}>
        <div class="settings-pane">
          <div class="settings-pane-head">
            <span id="quick-share-title">{t("quick-share-title")}</span>
            <button type="button" class="icon-btn" aria-label={t("share-dialog-close")} onClick={closeQuickShare}>✕</button>
          </div>
          <div class="settings-pane-body">
            <Show when={busy() === "list"}><div class="query-export-note">{t("quick-share-loading")}</div></Show>
            <Show when={needsConnect()}>
              <div class="settings-section">{t("quick-share-connect-title")}</div>
              <QuickShareConnect onConnected={connected} />
            </Show>
            <Show when={!needsConnect() && listing()?.account}>{(account) =>
              <p class="settings-hint quick-share-account">{t("quick-share-connected-as", { login: account().login })}</p>
            }</Show>

            <Show when={busy() === "prepare"}><div class="query-export-note">{t("quick-share-preparing")}</div></Show>
            <Show when={preview()}>{(shown) => <div class="share-review quick-share-preview" aria-live="polite">
              <div class="query-export-summary">{t("share-dialog-pages", { count: shown().pages.length })}</div>
              <ul class="query-export-pages share-pages"><For each={shown().pages}>{(page) =>
                <li>{page.name} <small>{page.path}</small>{page.journal ? t("query-export-dialog-journal") : ""}</li>
              }</For></ul>
              <Show when={shown().leftOut.length > 0}>
                <div class="query-export-note query-export-left-out">{t("query-export-dialog-left-out", { count: shown().leftOut.length })}
                  <ul class="query-export-pages"><For each={shown().leftOut}>{(page) =>
                    <li>{page.name} <small>{page.reason === "private" ? t("query-export-dialog-left-out-private") : t("query-export-dialog-left-out-exports-page")}</small></li>
                  }</For></ul>
                </div>
              </Show>
              <p class="quick-share-disclosure">{t("quick-share-disclosure")}</p>
              <p class="settings-hint">{t("quick-share-not-included")}</p>
              <p class="settings-hint quick-share-size">{t("quick-share-size", { size: quickShareSize(shown().envelopeBytes) })}</p>
              <Show when={!done()}>
                <p class="settings-hint quick-share-as-shown">{t("quick-share-as-shown")}
                  <button type="button" class="export-btn-secondary quick-share-refresh" disabled={!!busy() || needsConnect()}
                    onClick={() => void prepare()}>{t("quick-share-refresh")}</button></p>
              </Show>
            </div>}</Show>

            <Show when={created()}>{(share) => <div class="quick-share-created" role="status">
              <p>{copied() ? t("quick-share-copied") : t("quick-share-created")}</p>
              <p class="settings-hint quick-share-link">{t("quick-share-link-masked", { login: share().login, gist: share().gistId })}</p>
              <p class="settings-hint">{t("quick-share-key-on-device")}</p>
            </div>}</Show>
            <Show when={recorded()}>{(page) => <p class="settings-hint quick-share-recorded" role="status">{t("quick-share-definition-saved", { page: page() })}</p>}</Show>
            <Show when={checkNote()}><p class="settings-hint quick-share-check-note" role="status">{checkNote()}</p></Show>
            <Show when={error()}><div role="alert" class="query-export-refused quick-share-error">{error()}</div></Show>
          </div>
          <DialogActions class="share-foot"
            cancel={<button type="button" class="export-btn-secondary" onClick={closeQuickShare}>{done() ? t("quick-share-done") : t("share-dialog-cancel")}</button>}
            primary={<>
            <Show when={pending()}>
              <button type="button" class="export-btn-primary quick-share-check" disabled={!!busy()} onClick={() => void check()}>{t("quick-share-check")}</button>
            </Show>
            <Show when={created() ?? live()}>
              <button type="button" class="export-btn-primary quick-share-copy" disabled={!!busy()}
                onClick={() => void copyLink(created()?.shareId ?? live()!)}>{t("quick-share-copy-again")}</button>
            </Show>
            <Show when={!done() && !pending()}>
              <button type="button" class="export-btn-primary quick-share-create" disabled={!prepareId() || !!busy() || needsConnect()}
                onClick={() => void create()}>{busy() === "create" ? t("quick-share-creating") : t("quick-share-create")}</button>
            </Show>
            </>} />
        </div>
      </div>
    </div>
  );
}
