import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { t, msg } from "../i18n";
import { bindingOwner, readOwned, readOwnedResource, writeOwned } from "../owned";
import { graphScopedSignal, refuseStaleWrite } from "../binding";
import { EXPORT_PROPERTY, SHARE_PROPERTY } from "../exportsPage";
import { pendingRecordId, shareGistId } from "../quickShare/definitions";
import { QUICK_SHARE_OFF, quickShareErrorText, type QuickSharePrepared } from "../quickShare/commands";
import type { QueryPublicationRequest, QueryPublicationPlan } from "../types";
import { exportSheets } from "../sheet/exportSheets";
import { initQueryExportBudget, queryExportBudgetBytes } from "../queryExportBudget";
import { queryMacroExtents } from "../editor/queryMacro";
import { queryWorkspaceDsl } from "./queryWorkspaceDsl";
import { registerTransientLayer } from "../transientLayers";
import { trapTab } from "./focusableElements";
import { pushToast } from "../toasts";
import { dbg } from "../debug";
import { FolderExportMembership } from "./FolderExportMembership";

const [batchOwner, setBatchOwner] = graphScopedSignal<object>("binding");

/** Only mounted on a page carrying the Exports marker, including renamed pages and other panes. */
export function QuickShareAll(): JSX.Element {
  const instance = {};
  const close = () => { if (batchOwner() === instance) setBatchOwner(null); };
  onCleanup(close);
  return <>
    <button type="button" class="export-btn-secondary quick-share-all" onClick={() => setBatchOwner(instance)}>{t("quick-share-all")}</button>
    <Show when={batchOwner() === instance}><BatchDialog close={close} target={instance} /></Show>
  </>;
}

interface Entry { title: string; prepared?: QuickSharePrepared; folder?: { request: QueryPublicationRequest; plan: QueryPublicationPlan }; note?: string; outcome?: string }

/** Preparation only reads. Confirm consumes the same one-use native previews as single Re-share, in order. */
function BatchDialog(props: { close: () => void; target: object }): JSX.Element {
  const [entries, setEntries] = createSignal<Entry[]>([]);
  const [busy, setBusy] = createSignal<"prepare" | "upload" | null>("prepare");
  const [confirmed, setConfirmed] = createSignal(false);
  const [acknowledged, setAcknowledged] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const pending = new Set<string>();
  let alive = true;
  const owner = bindingOwner(() => alive);
  const discard = async (id: string) => {
    pending.delete(id);
    try { await readOwned(bindingOwner(), backend().quickShareDiscard(id)); }
    catch (failure) { dbg(`quick share: discarding a batch preview failed: ${String(failure)}`); }
  };
  onCleanup(() => { alive = false; for (const id of pending) void discard(id); });
  const add = (entry: Entry) => setEntries(rows => [...rows, entry]);
  const prepare = async () => {
    try {
      const listed = await readOwned(owner, backend().quickShareList().catch(() => QUICK_SHARE_OFF));
      if (listed.kind === "stale") return;
      const parsed = await readOwned(owner, backend().parseQuery(`(or (property ${EXPORT_PROPERTY}) (property ${SHARE_PROPERTY}))`, "og"));
      if (parsed.kind === "stale") return;
      const result = await readOwned(owner, backend().queryRun(parsed.value.query, parsed.value.view));
      if (result.kind === "stale") return;
      if (result.value.anchor !== "block" || result.value.exceeded) throw new Error(t("quick-share-all-incomplete"));
      const seen = new Set<string>();
      await initQueryExportBudget();
      for (const group of result.value.groups) for (const block of group.blocks) {
        if (!owner()) return;
        const folder = block.properties?.find(([key]) => key.toLowerCase() === EXPORT_PROPERTY)?.[1].trim();
        if (folder) {
          const title = `${group.page} (${folder})`;
          try {
            const macros = queryMacroExtents(block.raw);
            if (macros.length !== 1) throw new Error(t("quick-share-all-query"));
            const source = queryWorkspaceDsl(block.raw.slice(macros[0].start, macros[0].end));
            const request: QueryPublicationRequest = { argument: source.argument, dialect: source.dialect,
              properties: block.properties ?? [], currentPage: group.page, hostBlockId: block.id,
              name: group.page, folder, replace: true, includeQueryText: false, assetBudgetBytes: queryExportBudgetBytes() };
            const plan = await readOwned(owner, backend().publishQueryPlan(request));
            if (plan.kind === "stale") return;
            if (seen.has(`folder:${plan.value.folder}`)) add({ title, note: t("folder-export-duplicate") });
            else if (!plan.value.pages.length) add({ title, note: t("folder-export-empty") });
            else { seen.add(`folder:${plan.value.folder}`); add({ title, folder: { request, plan: plan.value } }); }
          } catch (failure) { if (owner()) add({ title, note: quickShareErrorText(failure) }); }
        }
        const value = block.properties?.find(([key]) => key.toLowerCase() === SHARE_PROPERTY)?.[1].trim();
        if (!value) continue;
        const gist = shareGistId(value);
        const row = listed.value.shares.find(row => row.gistId === gist);
        const title = row?.title || `${group.page} (${value})`;
        if (!listed.value.enabled || !listed.value.account || listed.value.error) { add({ title, note: listed.value.error || t("quick-share-all-no-key") }); continue; }
        if (pendingRecordId(value)) { add({ title, note: t("quick-share-all-pending") }); continue; }
        if (!row) { add({ title, note: t("quick-share-all-no-key") }); continue; }
        if (seen.has(gist)) { add({ title, note: t("quick-share-all-duplicate") }); continue; }
        seen.add(gist);
        try {
          const macros = queryMacroExtents(block.raw);
          if (macros.length !== 1) throw new Error(t("quick-share-all-query"));
          const source = queryWorkspaceDsl(block.raw.slice(macros[0].start, macros[0].end));
          const answer = await readOwnedResource(owner, backend().quickSharePrepare({ argument: source.argument,
            dialect: source.dialect, properties: block.properties ?? [], currentPage: group.page, hostBlockId: block.id,
            name: row.title || group.page, includeQueryText: false }, row.shareId), preview => discard(preview.prepareId));
          if (answer.kind === "stale") return;
          pending.add(answer.value.prepareId);
          add({ title: answer.value.preview.title, prepared: answer.value });
        } catch (failure) {
          if (owner()) add({ title, note: quickShareErrorText(failure) });
        }
      }
    } catch (failure) {
      if (owner()) setError(quickShareErrorText(failure));
    } finally { if (owner()) setBusy(null); }
  };
  const confirm = async () => {
    if (batchOwner() !== props.target) return refuseStaleWrite(t("quick-share-all"));
    if (busy() || confirmed() || !owner() || (entries().some(entry => entry.folder?.plan.anchor === "block") && !acknowledged())) return;
    setConfirmed(true); setBusy("upload");
    for (const entry of entries()) {
      if (!owner()) break;
      if (entry.folder) {
        let outcome: string;
        try {
          const { request, plan } = entry.folder;
          const sheets = await exportSheets(undefined, { kind: "query", request });
          if (!owner()) break;
          const answer = await writeOwned(owner, backend().publishQuery(request, plan.fingerprint, sheets));
          if (answer.kind === "stale") break;
          outcome = t("query-export-dialog-exported", { pages: answer.value.pages, path: answer.value.path,
            retired: answer.value.retired ?? "", previous: answer.value.retired ? 1 : 0 });
          if (answer.value.warnings.length) outcome += ` ${answer.value.warnings.join(" ")}`;
        } catch (failure) {
          outcome = quickShareErrorText(failure);
          if (!owner()) { pushToast(msg("quick-share-toast-failed", { error: outcome }), "error"); break; }
        }
        setEntries(rows => rows.map(row => row === entry ? { ...row, outcome } : row));
      }
      const preview = entry.prepared;
      if (!preview || !pending.delete(preview.prepareId)) continue;
      let outcome: string;
      try {
        const answer = await writeOwned(owner, backend().quickShareCreate(preview.prepareId));
        if (answer.kind === "stale") break;
        outcome = t("quick-share-reshared");
      } catch (failure) {
        outcome = quickShareErrorText(failure);
        if (!owner()) { pushToast(msg("quick-share-toast-failed", { error: outcome }), "error"); break; }
      }
      setEntries(rows => rows.map(row => row === entry ? { ...row, outcome } : row));
    }
    // A retired graph/view must never start the rest of the uploads.
    for (const id of pending) void discard(id);
    if (owner()) setBusy(null);
  };
  let dialog!: HTMLDivElement;
  let overlay!: HTMLDivElement;
  onMount(() => {
    onCleanup(registerTransientLayer({ id: "quick-share-all", root: () => overlay, dismiss: () => { props.close(); return true; } }));
    queueMicrotask(() => dialog?.focus());
    void prepare();
  });
  return <div class="modal-overlay" ref={overlay} onClick={props.close}>
    <div ref={dialog} class="settings-modal share-dialog quick-share-all-dialog" role="dialog" aria-modal="true"
      aria-labelledby="quick-share-all-title" tabIndex={-1} onClick={event => event.stopPropagation()}
      onKeyDown={event => trapTab(event, dialog)}>
      <div class="settings-pane">
        <div class="settings-pane-head"><span id="quick-share-all-title">{t("quick-share-all")}</span>
          <button type="button" class="icon-btn quick-share-all-close" aria-label={t("share-dialog-close")} onClick={props.close}>✕</button></div>
        <div class="settings-pane-body" aria-live="polite">
          <p>{t("quick-share-all-review")}</p>
          <Show when={entries().some(entry => entry.prepared)}><p>{t("quick-share-revisions")}</p></Show>
          <Show when={busy()}><p role="status">{busy() === "prepare" ? t("exports-all-preparing") : t("exports-all-updating")}</p></Show>
          <Show when={error()}><p role="alert">{error()}</p></Show>
          <For each={entries()}>{entry => <section class="quick-share-all-entry">
            <strong>{entry.title}</strong>
            <Show when={entry.folder}>{folder => <>
              <p>{t("query-export-dialog-summary", { results: folder().plan.rowCount, pages: folder().plan.pages.length })}</p>
              <p>{folder().plan.path}</p>
              <FolderExportMembership plan={folder().plan} />
              <ul><For each={folder().plan.pages}>{page => <li>{page.name}</li>}</For></ul>
              <Show when={folder().plan.leftOut.length}>
                <p>{t("query-export-dialog-left-out", { count: folder().plan.leftOut.length })}</p>
                <ul><For each={folder().plan.leftOut}>{page => <li>{page.name}: {page.reason === "private" ? t("query-export-dialog-left-out-private") : t("query-export-dialog-left-out-exports-page")}</li>}</For></ul>
              </Show>
              <p>{t("folder-export-replace-review")}</p>
            </>}</Show>
            <Show when={entry.prepared?.membership}>{diff => <div class="quick-share-membership">
              <p>{t("quick-share-membership", { added: diff().added.length, removed: diff().removed.length })}</p>
              <details><summary>{t("quick-share-membership-pages")}</summary><ul>
                <For each={diff().added}>{name => <li>+ {name}</li>}</For>
                <For each={diff().removed}>{name => <li>− {name}</li>}</For>
              </ul></details>
            </div>}</Show>
            <Show when={entry.note}><p>{t("quick-share-all-skipped", { reason: entry.note! })}</p></Show>
            <Show when={entry.outcome}><p role="status">{entry.outcome}</p></Show>
          </section>}</For>
          <Show when={entries().some(entry => entry.folder?.plan.anchor === "block")}>
            <label><input type="checkbox" checked={acknowledged()} onChange={event => setAcknowledged(event.currentTarget.checked)} />{t("query-export-dialog-all-blocks-on-these-pages-will-be-exported")}</label>
          </Show>
          <Show when={!busy() && !entries().length && !error()}><p>{t("quick-share-all-empty")}</p></Show>
        </div>
        <div class="share-foot">
          <Show when={!confirmed()}><button type="button" class="export-btn-primary quick-share-all-confirm"
            disabled={!!busy() || !!error() || !entries().some(entry => entry.prepared || entry.folder)
              || (entries().some(entry => entry.folder?.plan.anchor === "block") && !acknowledged())} onClick={() => void confirm()}>{t("quick-share-all-confirm")}</button></Show>
        </div>
      </div>
    </div>
  </div>;
}
