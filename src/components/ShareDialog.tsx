import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup, onMount, untrack, type Accessor, type JSX } from "solid-js";
import { t, msg } from "../i18n";
import { DialogActions } from "./DialogActions";
import { backend } from "../backend";
import { bindingCurrent, captureBinding } from "../binding";
import { appendQueryBlock, blockProperty, blockWritable, setBlockProperty } from "../document";
import { QUERY_MACRO_NAMES } from "../editor/queryMacroName";
import { EXPORT_PROPERTY, ensureExportsPage, pageSelectorMacro, selectorPageName } from "../exportsPage";
import { graphOwner, readOwned } from "../owned";
import { readLatestOr, readOr } from "../resourceRead";
import { dbg } from "../debug";
import { pushToast } from "../toasts";
import { closeShareDialog, openQueryExportThen, openQuickShare, type ShareTarget } from "../shareState";
import { registerTransientLayer } from "../transientLayers";
import type { QueryPublicationRequest } from "../types";
import { QueryBuilder, type BuilderSession } from "./QueryBuilder";
import { QueryLivePreview } from "./QueryLivePreview";
import { trapTab } from "./focusableElements";
import { queryWorkspaceDsl } from "./queryWorkspaceDsl";
import { QUICK_SHARE_OFF } from "../quickShare/commands";

/** Quiet time before a changed selection is reviewed again (the live preview's own window). */
export const SHARE_REVIEW_DEBOUNCE_MS = 250;
const LAYER_ID = "share-dialog";

/** The share dialog (share step 1, SHARE-DIALOG.md §3): what to share, a review of the pages that go out, and
 *  where. Folder export reuses the query export's destination and replace flow with the reviewed request, then
 *  saves the definition (`export:: <folder>`) on the exports page, or on the query block it started from. Link
 *  hands the reviewed request to the quick share dialog (DESIGN §3.9), offered only where quick share is on. */
export function ShareDialog(props: { target: Accessor<ShareTarget | null> }): JSX.Element {
  return <Show when={props.target()}>{(target) => <Dialog target={target()} />}</Show>;
}

type Mode = "page" | "query";
type Where = "folder" | "link";

function Dialog(props: { target: ShareTarget }): JSX.Element {
  const seed = props.target;
  const block = "query" in seed ? seed : null;
  const hostBlockId = block?.query.hostBlockId ?? null;
  const contextPage = "page" in seed ? seed.page : (seed.query.currentPage ?? "");
  const [pageName, setPageName] = createSignal(contextPage);
  // From a query block the query is preselected (seeded with the block's text); otherwise this page.
  const [mode, setMode] = createSignal<Mode>(block || !contextPage ? "query" : "page");
  const [edited, setEdited] = createSignal<string | undefined>(undefined);
  const [includeQueryText, setIncludeQueryText] = createSignal(false);
  const [where, setWhere] = createSignal<Where>("folder");
  // Quick share is gated off in shipped builds until its viewer is deployed (DESIGN §5 phase 3).
  const [quickShare] = createResource(() => backend().quickShareList());
  const linkAvailable = () => readOr(quickShare, QUICK_SHARE_OFF, "quick share listing")?.enabled === true;
  let mounted = true;
  onCleanup(() => { mounted = false; });

  // The "this page" selector, printed by the engine so a definition stays byte-stable under the builder.
  const [pageMacro] = createResource(pageName, async (name) => name ? pageSelectorMacro(name) : null);
  const selector = () => readOr(pageMacro, null, "page selector");
  const queryMacro = (): string | null => edited() ?? (block ? block.macro : selector());
  const macro = (): string | null => (mode() === "page" ? selector() : queryMacro());

  // Re-opening a definition shows the same selection: a block whose query is exactly a page's selector opens as "This page".
  onMount(() => {
    if (!block) return;
    const owner = graphOwner(() => mounted);
    const source = queryWorkspaceDsl(block.macro);
    void readOwned(owner, backend().parseQuery(source.argument, source.dialect)).then(async (parsed) => {
      if (parsed.kind !== "current" || !owner()) return;
      const name = selectorPageName(parsed.value.query);
      if (name === null || (await pageSelectorMacro(name)) !== block.macro.trim() || !owner() || edited() !== undefined) return;
      setPageName(name);
      setMode("page");
    // A selector that cannot be recognized stays a query: the text itself is shown and exported unchanged.
    }).catch((error) => dbg(`share dialog: recognizing a page selector failed: ${String(error)}`));
  });

  const unchanged = () => !!block && macro()?.trim() === block.macro.trim();
  const request = createMemo<QueryPublicationRequest | undefined>(() => {
    const text = macro();
    if (!text) return undefined;
    if (unchanged()) return { ...block!.query, includeQueryText: false };
    const source = queryWorkspaceDsl(text);
    const name = mode() === "page" ? pageName() : (block?.query.name || pageName() || contextPage);
    return { argument: source.argument, dialect: source.dialect, properties: [], currentPage: contextPage || null,
      hostBlockId: null, name: name || "Query", includeQueryText: false };
  });

  // The review runs after a quiet window, except for the first answer.
  const [settled, setSettled] = createSignal<QueryPublicationRequest | undefined>(untrack(request));
  createEffect(() => {
    const next = request();
    const current = untrack(settled);
    if (JSON.stringify(next) === JSON.stringify(current)) return;
    if (!current || !next) { setSettled(next); return; }
    const timer = setTimeout(() => setSettled(next), SHARE_REVIEW_DEBOUNCE_MS);
    onCleanup(() => clearTimeout(timer));
  });
  const [plan] = createResource(settled, (req) => backend().publishQueryPlan(req));
  const reviewed = () => readOr(plan, undefined, "share plan");
  const waiting = () => plan.loading || JSON.stringify(settled()) !== JSON.stringify(request());
  const canExport = () => !!reviewed()?.pages.length && !plan.error && !waiting() && !!request();

  // ---- query mode: the builder edits the query text; the preview runs what the builder shows ----
  const edits = { n: 0 };
  const [builderSession, { mutate }] = createResource(
    () => (mode() === "query" ? queryMacro() : null),
    async (text): Promise<BuilderSession | undefined> => {
      const owner = graphOwner(() => mounted);
      const source = queryWorkspaceDsl(text);
      const parsed = await readOwned(owner, backend().parseQuery(source.argument, source.dialect));
      return parsed.kind === "current" && owner() ? { query: parsed.value.query, view: parsed.value.view }
        : readLatestOr(builderSession, undefined, "query text");
    });
  const [buildError, setBuildError] = createSignal<string | null>(null);
  const applyBuilderEdit = async (next: BuilderSession) => {
    const ticket = ++edits.n;
    mutate(next);
    const owner = graphOwner(() => mounted && ticket === edits.n);
    try {
      const source = queryWorkspaceDsl(queryMacro() ?? "");
      let printed: string;
      if (source.dialect === "macro_tql") {
        const answer = await readOwned(owner, backend().printQuery(next.query, next.view, "tql_macro"));
        if (answer.kind === "stale" || !owner()) return;
        printed = `{{${source.name} ${answer.value}}}`;
      } else {
        // OG text stays OG when it can say the edit; otherwise it becomes a Tine query (D-18).
        const expressible = await readOwned(owner, backend().queryOgExpressible(next.query, next.view));
        if (expressible.kind === "stale" || !owner()) return;
        const answer = await readOwned(owner, backend().printQuery(next.query, next.view, expressible.value ? "og" : "tql_macro"));
        if (answer.kind === "stale" || !owner()) return;
        printed = !expressible.value ? `{{${QUERY_MACRO_NAMES[1]} ${answer.value}}}`
          : source.wrapped ? `{{${source.name} ${answer.value}}}` : `{{${QUERY_MACRO_NAMES[0]} ${answer.value}}}`;
      }
      setBuildError(null);
      setEdited(printed);
    } catch (failure) {
      if (owner()) setBuildError(failure instanceof Error ? failure.message : String(failure));
    }
  };
  const previewContext = () => (contextPage ? { current_page: contextPage } : undefined);

  // ---- the export, then the definition ----
  const existingFolder = () => (hostBlockId ? blockProperty(hostBlockId, EXPORT_PROPERTY) : null);
  const exportNow = () => {
    const base = request();
    const text = macro();
    if (!base || !text || !canExport()) return;
    const asBlock = unchanged() && !!hostBlockId;
    if (where() === "link" && linkAvailable()) {
      closeShareDialog();
      // The created share's `tine.share::` goes where a folder export's `export::` would (D-20261018-Q1).
      openQuickShare({ ...base, includeQueryText: includeQueryText() }, { hostBlockId: asBlock ? hostBlockId : null, macro: text });
      return;
    }
    const binding = captureBinding();
    const saveDefinition = async ({ folder }: { folder: string }) => {
      if (!bindingCurrent(binding)) return;
      try {
        if (asBlock) {
          if (!blockWritable(hostBlockId!)) throw new Error("the query block can't be edited");
          if (blockProperty(hostBlockId!, EXPORT_PROPERTY) !== folder) setBlockProperty(hostBlockId!, EXPORT_PROPERTY, folder);
          return;
        }
        const page = await ensureExportsPage();
        if (!bindingCurrent(binding)) return;
        if (!(await appendQueryBlock(page, text, [[EXPORT_PROPERTY, folder]]))) throw new Error(`couldn't write to ${page}`);
        if (bindingCurrent(binding)) pushToast(msg("share-dialog-definition-saved", { page }), "success");
      } catch (error) {
        if (bindingCurrent(binding)) pushToast(msg("share-dialog-definition-failed", { error: error instanceof Error ? error.message : String(error) }), "error");
      }
    };
    closeShareDialog();
    openQueryExportThen({ ...base, includeQueryText: includeQueryText() }, saveDefinition);
  };

  // ---- shell: transient layer, focus trap ----
  let dialog!: HTMLDivElement;
  let overlay!: HTMLDivElement;
  onMount(() => {
    onCleanup(registerTransientLayer({ id: LAYER_ID, root: () => overlay ?? null, dismiss: () => { closeShareDialog(); return true; } }));
    queueMicrotask(() => dialog?.focus());
  });

  return (
    <div class="modal-overlay" ref={overlay} onClick={closeShareDialog}>
      <div ref={dialog} class="settings-modal share-dialog" role="dialog" aria-modal="true" aria-labelledby="share-dialog-title"
        tabIndex={-1} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => trapTab(event, dialog)}>
        <div class="settings-pane">
          <div class="settings-pane-head">
            <span id="share-dialog-title">{t("share-dialog-title")}</span>
            <button type="button" class="icon-btn" aria-label={t("share-dialog-close")} onClick={closeShareDialog}>✕</button>
          </div>
          <div class="settings-pane-body">
            <div class="settings-section">{t("share-dialog-what")}</div>
            <div class="share-choices" role="radiogroup" aria-label={t("share-dialog-what")}>
              <label class="share-choice" classList={{ disabled: !pageName() }}>
                <input type="radio" name="share-what" value="page" checked={mode() === "page"} disabled={!pageName()}
                  onChange={() => setMode("page")} />
                <span class="share-choice-label">{t("share-dialog-this-page")}</span>
                <Show when={pageName()}><small class="settings-hint">{t("share-dialog-this-page-note", { page: pageName() })}</small></Show>
              </label>
              <label class="share-choice disabled" aria-disabled="true">
                <input type="radio" name="share-what" value="neighbours" disabled />
                <span class="share-choice-label">{t("share-dialog-neighbours")}</span>
                <small class="settings-hint">{t("share-dialog-neighbours-soon")}</small>
              </label>
              <label class="share-choice">
                <input type="radio" name="share-what" value="query" checked={mode() === "query"} onChange={() => setMode("query")} />
                <span class="share-choice-label">{t("share-dialog-query")}</span>
              </label>
            </div>
            <Show when={mode() === "query"}>
              <div class="share-query">
                <p class="settings-hint">{t("share-dialog-query-note")}</p>
                <QueryBuilder
                  session={() => readLatestOr(builderSession, undefined, "query text")}
                  onChange={(next) => void applyBuilderEdit(next)}
                  paneDialect={queryWorkspaceDsl(queryMacro() ?? "").dialect === "macro_tql" ? "tql" : "og"}
                  sheetAlwaysOpen
                  previewContext={previewContext}
                  parentTransientId={LAYER_ID}
                />
                <Show when={buildError()}><div role="alert" class="query-export-refused">{buildError()}</div></Show>
                <QueryLivePreview
                  query={() => readLatestOr(builderSession, undefined, "query text")?.query}
                  view={() => readLatestOr(builderSession, undefined, "query text")?.view ?? {}}
                  context={previewContext}
                />
              </div>
            </Show>

            <div class="settings-section">{t("share-dialog-review")}</div>
            <div class="share-review" aria-live="polite">
              <Show when={waiting() && !plan.error}><div class="query-export-note">{t("share-dialog-resolving")}</div></Show>
              <Show when={plan.error}><div role="alert" class="query-export-refused">{String(plan.error)}</div></Show>
              <Show when={reviewed() && !plan.error}>{(_) => <>
                <Show when={reviewed()!.pages.length > 0}>
                  <div class="query-export-summary">{t("share-dialog-pages", { count: reviewed()!.pages.length })}</div>
                  <ul class="query-export-pages share-pages"><For each={reviewed()!.pages}>{(page) =>
                    <li>{page.name} <small>{page.path}</small>{page.journal ? t("query-export-dialog-journal") : ""}</li>
                  }</For></ul>
                </Show>
                <Show when={reviewed()!.leftOut.length > 0}>
                  <div class="query-export-note query-export-left-out">{t("query-export-dialog-left-out", { count: reviewed()!.leftOut.length })}
                    <ul class="query-export-pages"><For each={reviewed()!.leftOut}>{(page) =>
                      <li>{page.name} <small>{page.reason === "private" ? t("query-export-dialog-left-out-private") : t("query-export-dialog-left-out-exports-page")}</small></li>
                    }</For></ul>
                  </div>
                </Show>
                <Show when={reviewed()!.selectsHostPage}>
                  <div class="query-export-note query-export-host-note">{t("query-export-dialog-selects-host-page")}</div>
                </Show>
              </>}</Show>
              <label class="query-export-ack query-export-include-query"><input type="checkbox" checked={includeQueryText()}
                onChange={(event) => setIncludeQueryText(event.currentTarget.checked)} />{t("query-export-dialog-include-query-text")}</label>
            </div>

            <div class="settings-section">{t("share-dialog-where")}</div>
            <div class="share-choices" role="radiogroup" aria-label={t("share-dialog-where")}>
              <label class="share-choice">
                <input type="radio" name="share-where" value="folder" checked={where() === "folder"} onChange={() => setWhere("folder")} />
                <span class="share-choice-label">{t("share-dialog-folder")}</span>
                <small class="settings-hint">{t("share-dialog-folder-note")}</small>
              </label>
              <label class="share-choice" classList={{ disabled: !linkAvailable() }} aria-disabled={linkAvailable() ? undefined : "true"}>
                <input type="radio" name="share-where" value="link" checked={where() === "link"} disabled={!linkAvailable()}
                  onChange={() => setWhere("link")} />
                <span class="share-choice-label">{t("share-dialog-link")}</span>
                <small class="settings-hint">{linkAvailable() ? t("share-dialog-link-note") : t("share-dialog-link-soon")}</small>
              </label>
            </div>
            <Show when={where() === "folder"}>
              <p class="settings-hint share-save-note">{unchanged() && hostBlockId ? t("share-dialog-saved-on-block") : t("share-dialog-saved-as-definition")}</p>
            </Show>
            <Show when={where() === "link" && linkAvailable()}>
              <p class="settings-hint share-save-note">{unchanged() && hostBlockId ? t("share-dialog-link-saved-on-block") : t("share-dialog-link-saved-as-definition")}</p>
            </Show>
            <Show when={where() === "folder" && unchanged() && existingFolder()}>
              <p class="settings-hint share-existing">{t("share-dialog-already-exported", { folder: existingFolder()! })}</p>
            </Show>
          </div>
          <DialogActions
            class="share-foot"
            cancel={<button type="button" class="export-btn-secondary" onClick={closeShareDialog}>{t("share-dialog-cancel")}</button>}
            primary={<button type="button" class="export-btn-primary share-export" disabled={!canExport()} onClick={exportNow}>{where() === "link" ? t("share-dialog-create-link") : t("share-dialog-export")}</button>} />
        </div>
      </div>
    </div>
  );
}
