import { Rich } from "../i18n/Rich";
import { t, msg, rawText } from "../i18n";
import { For, Show, createResource, createSignal, onCleanup, onMount, type Accessor, type JSX } from "solid-js";
import { backend } from "../backend";
import { exportSheets } from "../sheet/exportSheets";
import { closeQueryExport, openSettings } from "../ui";
import { pushToast } from "../toasts";
import { bindingOwner, writeOwned } from "../owned";
import type { QueryPublicationRequest } from "../types";
import { readOr } from "../resourceRead";
import { initQueryExportBudget, queryExportBudgetBytes } from "../queryExportBudget";
import { activeWindow, listen } from "../windowRealm";
import { registerTransientLayer } from "../transientLayers";

/** Review complete owner pages, then publish a graph query leaf as a static
 * site and read-only browser app, preserving/reporting replaced output. The backend rechecks the fingerprint
 * before writing, so a graph edit between review and confirm is a refusal. */
export function QueryExportDialog(props: { request: Accessor<QueryPublicationRequest | null> }): JSX.Element {
  return <Show when={props.request()}>{(request) => <Dialog request={request()} />}</Show>;
}

function Dialog(props: { request: QueryPublicationRequest }): JSX.Element {
  const [name, setName] = createSignal(props.request.name);
  const [plannedName, setPlannedName] = createSignal(props.request.name);
  const [destination, setDestination] = createSignal<"create" | "replace" | "separate">("create");
  const [acknowledged, setAcknowledged] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [overBudget, setOverBudget] = createSignal(false);
  let mounted = true;
  onCleanup(() => { mounted = false; });
  const [plan] = createResource(plannedName, async (value) => {
    if (!value.trim()) throw new Error("Give the export a name.");
    await initQueryExportBudget();
    return backend().publishQueryPlan({ ...props.request, name: value });
  });
  const reviewed = () => readOr(plan, undefined, "export plan");
  const folder = () => destination() === "separate" ? reviewed()?.suggestedFolder : reviewed()?.folder;
  const canExport = () => !!reviewed()?.pages.length && !busy() && (!reviewed()!.exists || destination() !== "create")
    && (reviewed()!.anchor !== "block" || acknowledged());
  const changeName = () => { setDestination("create"); setAcknowledged(false); setPlannedName(name().trim()); };
  const publish = async () => {
    const selection = reviewed();
    if (!selection || !folder() || !canExport()) return;
    const owner = bindingOwner(() => mounted);
    setBusy(true);
    setError(""); setOverBudget(false);
    try {
      const request = { ...props.request, name: plannedName(), folder: folder(), replace: destination() === "replace", assetBudgetBytes: queryExportBudgetBytes() };
      const sheets = await exportSheets(undefined, { kind: "query", request });
      if (!owner()) return;
      const receipt = await writeOwned(owner, backend().publishQuery(request, selection.fingerprint, sheets));
      if (receipt.kind === "current") {
        closeQueryExport();
        pushToast(msg("query-export-dialog-exported", { pages: String(receipt.value.pages), path: receipt.value.path, retired: receipt.value.retired ?? "", previous: receipt.value.retired ? 1 : 0 }), "success", { sticky: true });
        if (receipt.value.warnings.length) pushToast(rawText(receipt.value.warnings.join(" ")), "warn", { sticky: true });
      }
    } catch (cause) {
      if (owner()) {
        setError(String((cause as Error)?.message ?? cause));
        setOverBudget(typeof cause === "object" && cause !== null && "kind" in cause && cause.kind === "assetBudget");
      }
    } finally {
      if (owner()) setBusy(false);
    }
  };
  onMount(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); closeQueryExport(); }
    };
    onCleanup(listen(activeWindow(), "keydown", key, true));
    // An open app-level layer pins the overlay window (overlayWindowId), so
    // focusing another Tine window does not remount the dialog there and
    // discard its form (review F10).
    onCleanup(registerTransientLayer({
      id: "query-export",
      root: () => overlay ?? null,
      dismiss: () => { closeQueryExport(); return true; },
    }));
  });
  let overlay: HTMLDivElement | undefined;
  return (
    <div class="modal-overlay" ref={overlay} onClick={closeQueryExport}>
      <div class="export-modal query-export-modal" role="dialog" aria-label={t("query-export-dialog-export-query-results")} onClick={(event) => event.stopPropagation()}>
        <div class="export-head">{t("query-export-dialog-export-query-results")}</div>
        <div class="export-opts">
          <label class="export-opt-row"><span class="export-opt-label">{t("query-export-dialog-name")}</span>
            <input value={name()} onInput={(event) => setName(event.currentTarget.value)}
              onBlur={changeName}
              onKeyDown={(event) => { event.stopPropagation(); if (event.key === "Enter") changeName(); }} />
          </label>
          <Show when={plan.loading}><div class="query-export-note">{t("query-export-dialog-resolving-pages")}</div></Show>
          <Show when={plan.error}><div role="alert" class="query-export-refused">{String(plan.error)}</div></Show>
          <Show when={reviewed()}>{(selection) => <>
            <div class="query-export-summary">{t("query-export-dialog-summary", { results: String(selection().rowCount), pages: String(selection().pages.length) })}</div>
            <ul class="query-export-pages"><For each={selection().pages}>{(page) =>
              <li>{page.name} <small>{page.path}</small>{page.journal ? t("query-export-dialog-journal") : ""}</li>
            }</For></ul>
            <Show when={selection().anchor === "block" && selection().pages.length > 0}>
              <label class="query-export-ack"><input type="checkbox" checked={acknowledged()}
                onChange={(event) => setAcknowledged(event.currentTarget.checked)} />{t("query-export-dialog-all-blocks-on-these-pages-will-be-exported")}</label>
            </Show>
            <Show when={selection().exists}>
              <div role="group" aria-label={t("query-export-dialog-destination")}><Rich message={msg("query-export-dialog-existing", { folder: selection().folder })} /><label><input type="radio" name="query-export-destination" checked={destination() === "replace"}
                  onChange={() => setDestination("replace")} />{t("query-export-dialog-replace-it-the-previous-export-is-kept-in")}</label>
                <label><input type="radio" name="query-export-destination" checked={destination() === "separate"}
                  onChange={() => setDestination("separate")} /><Rich message={msg("query-export-dialog-separate", { folder: selection().suggestedFolder ?? "" })} /></label>
              </div>
            </Show>
            <div class="query-export-note"><Rich message={msg("query-export-dialog-path", { path: selection().path.slice(0, -selection().folder.length) + (folder() ?? selection().folder) })} /></div>
          </>}</Show>
          <Show when={error()}><div role="alert" class="query-export-refused">{error()}
            <Show when={overBudget()}><button class="export-btn-secondary" onClick={() => { closeQueryExport(); openSettings("graph"); }}>{t("query-export-dialog-adjust-limit-in-settings")}</button></Show>
          </div></Show>
        </div>
        <div class="export-foot">
          <button class="export-btn-secondary" onClick={closeQueryExport}>{t("query-export-dialog-cancel")}</button>
          <button class="export-btn-primary" disabled={!canExport()}
            onClick={() => void publish()}>{busy() ? t("query-export-dialog-exporting") : t("query-export-dialog-export")}</button>
        </div>
      </div>
    </div>
  );
}
