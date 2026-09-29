import { For, Show, createResource, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { closeQueryExport, queryExportRequest } from "../ui";
import { pushToast } from "../toasts";
import type { QueryPublicationRequest } from "../types";

/** Review complete owner pages, then pick an external folder for a create-only
 * static site and read-only browser app. The backend rechecks the fingerprint
 * before writing, so a graph edit between review and confirm is a refusal. */
export function QueryExportDialog(): JSX.Element {
  return <Show when={queryExportRequest()}>{(request) => <Dialog request={request()} />}</Show>;
}

function Dialog(props: { request: QueryPublicationRequest }): JSX.Element {
  const [name, setName] = createSignal(props.request.name);
  const [plannedName, setPlannedName] = createSignal(props.request.name);
  const [destination, setDestination] = createSignal<string | null>(null);
  const [acknowledged, setAcknowledged] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const [plan] = createResource(plannedName, async (value) => {
    if (!value.trim()) throw new Error("Give the export a name.");
    return backend().publishQueryPlan({ ...props.request, name: value });
  });
  const reviewed = () => plan.error === undefined ? plan() : undefined;
  const choose = async () => {
    const selected = await backend().pickFolder("Choose a folder outside the graph for this export");
    if (selected) setDestination(selected);
  };
  const publish = async () => {
    const selection = reviewed();
    const parent = destination();
    if (!selection || !parent || busy() || (selection.anchor === "block" && !acknowledged())) return;
    setBusy(true);
    setError("");
    try {
      const receipt = await backend().publishQuery({ ...props.request, name: plannedName() }, selection.fingerprint, parent);
      closeQueryExport();
      pushToast(`Exported ${receipt.pages} pages to ${receipt.path}`, "success", { sticky: true });
    } catch (cause) {
      setError(String((cause as Error)?.message ?? cause));
    } finally {
      setBusy(false);
    }
  };
  onMount(() => {
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); closeQueryExport(); }
    };
    window.addEventListener("keydown", key, true);
    onCleanup(() => window.removeEventListener("keydown", key, true));
  });
  return (
    <div class="modal-overlay" onClick={closeQueryExport}>
      <div class="export-modal query-export-modal" role="dialog" aria-label="Export query results" onClick={(event) => event.stopPropagation()}>
        <div class="export-head">Export query results</div>
        <div class="export-opts">
          <label class="export-opt-row"><span class="export-opt-label">Name</span>
            <input value={name()} onInput={(event) => setName(event.currentTarget.value)}
              onBlur={() => setPlannedName(name().trim())}
              onKeyDown={(event) => { event.stopPropagation(); if (event.key === "Enter") setPlannedName(name().trim()); }} />
          </label>
          <Show when={plan.loading}><div class="query-export-note">Resolving pages…</div></Show>
          <Show when={plan.error}><div role="alert" class="query-export-refused">{String(plan.error)}</div></Show>
          <Show when={reviewed()}>{(selection) => <>
            <div class="query-export-summary">{selection().rowCount} results on {selection().pages.length} complete pages</div>
            <ul class="query-export-pages"><For each={selection().pages}>{(page) =>
              <li>{page.name} <small>{page.path}</small>{page.journal ? " · journal" : ""}</li>
            }</For></ul>
            <Show when={selection().anchor === "block" && selection().pages.length > 0}>
              <label class="query-export-ack"><input type="checkbox" checked={acknowledged()}
                onChange={(event) => setAcknowledged(event.currentTarget.checked)} />
                All blocks on these pages will be exported, including blocks that did not match.
              </label>
            </Show>
            <div class="query-export-note">The selected folder receives a new <code>{selection().folder}</code> directory.
              The export includes a static site and a read-only app. It is not uploaded.</div>
          </>}</Show>
          <button class="export-btn-secondary" type="button" onClick={() => void choose()}>Choose destination…</button>
          <Show when={destination()}><div class="query-export-note">Destination: {destination()}</div></Show>
          <Show when={error()}><div role="alert" class="query-export-refused">{error()}</div></Show>
        </div>
        <div class="export-foot">
          <button class="export-btn-secondary" onClick={closeQueryExport}>Cancel</button>
          <button class="export-btn-primary" disabled={!reviewed() || !reviewed()!.pages.length || !destination() || busy() || (reviewed()!.anchor === "block" && !acknowledged())}
            onClick={() => void publish()}>{busy() ? "Exporting…" : "Export"}</button>
        </div>
      </div>
    </div>
  );
}
