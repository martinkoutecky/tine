import { msg, t } from "../i18n";
import { DialogActions } from "./DialogActions";
// The unsaved-changes recovery panel (GH #540, master dc3f2104b). Opened when the
// user declines to discard at close, or from a failed-save toast. It lists every
// page whose edits are not on disk and offers the ways out that never lose text:
// retry the ordinary save, open the page (its conflict bar resolves it), or copy
// the draft. Copying is read-only and never acknowledges a save.
import { For, Show, createEffect, createSignal, on, onCleanup, onMount, type JSX } from "solid-js";
import { flushAll, hostScope, hostSession, saveStep, unsavedDrafts, type KeptOriginal, type RestoreOutcome } from "../document";
import { backend } from "../backend";
import { bindingCurrent, captureBinding, type Binding } from "../binding";
import { pushToast } from "../toasts";
import { bindingOwner, latestOwner, readOwned, writeOwned } from "../owned";
import { reportUiFailure } from "../uiFailure";
import { mainPaneRouter } from "../router";
import { writeClipboardText } from "../clipboard";
import { closeUnsavedRecovery, setKeptOriginalCount, unsavedRecoveryOpen } from "../unsavedRecovery";
import { registerTransientLayer } from "../transientLayers";
import { DEFAULT_EXPORT_OPTIONS, exportOutline, type ExportNode } from "../editor/exportText";
import type { PageDto } from "../types";
import "../styles/unsavedRecovery.css";

/** The page as source text: markup and properties kept, nothing expanded. */
export function recoveryDraftText(page: PageDto): string {
  const nodes = (blocks: PageDto["blocks"]): ExportNode[] => blocks.map((block) => ({
    raw: block.raw, format: page.format ?? "md", children: nodes(block.children),
  }));
  const outline = exportOutline(nodes(page.blocks), { ...DEFAULT_EXPORT_OPTIONS, content: "source" });
  return (page.pre_block === null ? "" : `${page.pre_block}\n`) + outline;
}

function RecoveryDraft(props: { page: PageDto }): JSX.Element {
  const [message, setMessage] = createSignal("");
  const copy = async (complete: boolean) => {
    try {
      await writeClipboardText(complete ? JSON.stringify(props.page, null, 2) : recoveryDraftText(props.page));
      setMessage("Copied. Paste it into a separate file to keep a recovery copy.");
    } catch {
      setMessage("Could not copy. Select the draft below and copy it manually. Your draft remains here.");
    }
  };
  return <div class="recovery-draft">
    <pre tabIndex={0}>{recoveryDraftText(props.page)}</pre>
    <button class="settings-btn" type="button" onClick={() => void copy(false)}>{t("unsaved-recovery-copy-draft")}</button>
    <button class="settings-btn" type="button" onClick={() => void copy(true)}>{t("unsaved-recovery-copy-complete-recovery-data")}</button>
    <p role="status">{message()}</p>
  </div>;
}

/** The one report of a restore's outcome (GH #540): one toast per Restore, none
 * once the session that asked is gone, and none for a save the page's own editor
 * reports (`unwritten.held`). */
export function reportRestore(outcome: RestoreOutcome, path: string): void {
  switch (outcome.kind) {
    case "restored": pushToast(msg("unsaved-recovery-restored", { path }), "info"); return;
    case "superseded": pushToast(msg("unsaved-recovery-restore-superseded", { path }), "info"); return;
    case "unwritten": if (!outcome.held) pushToast(msg("unsaved-recovery-restore-unwritten", { path }), "error"); return;
    case "uncertain": pushToast(msg("unsaved-recovery-restore-uncertain", { path }), "error"); return;
  }
  switch (outcome.reason) {
    case "notKept": pushToast(msg("unsaved-recovery-restore-not-kept"), "info"); return;
    case "ineligible": pushToast(msg("unsaved-recovery-restore-ineligible", { path }), "error"); return;
    case "held": pushToast(msg("unsaved-recovery-restore-held", { name: outcome.name }), "info"); return;
    case "unsaved": pushToast(msg("unsaved-recovery-restore-unsaved", { path }), "info"); return;
    case "occupied": pushToast(msg("unsaved-recovery-restore-occupied", { path }), "error"); return;
    case "twin": pushToast(msg("unsaved-recovery-restore-twin", { path, file: outcome.file }), "error"); return;
    case "failed": pushToast(msg("unsaved-recovery-restore-failed", { path, step: saveStep(outcome) }), "error"); return;
    case "unavailable": pushToast(msg("unsaved-recovery-restore-unavailable"), "error"); return;
  }
}

/** The list one host session gave, which its rows carry. */
interface KeptList { session: number; rows: KeptOriginal[] }

/** The originals a cancelled rename kept, each with Restore and Copy original
 * (GH #540). Each mounted panel lists them itself: when it opens, after each
 * Restore, and whenever a (re)bind moves the host scope. A reply applies only to
 * this panel's latest request, in the scope and graph binding it was asked in;
 * every command carries the session that listed its row, which the host checks. */
function KeptOriginals(props: { binding: Binding }): JSX.Element {
  const [list, setList] = createSignal<KeptList | null>(null);
  /** Rows this panel restored, or found holding other content: Open, until the
   * panel closes. */
  const [restored, setRestored] = createSignal<{ row: KeptOriginal; other: boolean }[]>([]);
  /** This panel's own request sequence (`latestOwner`): a reply applies only to
   * its newest list request. */
  const requests = {};
  let mounted = true;
  onCleanup(() => { mounted = false; });
  const relist = async () => {
    const scope = hostScope();
    const session = hostSession();
    const owner = latestOwner(requests, "kept-originals",
      bindingOwner(() => mounted && scope === hostScope() && bindingCurrent(props.binding)));
    if (session === null || !owner()) return setList(null);
    try {
      const rows = await readOwned(owner, backend().pageKeptOriginals(session));
      if (rows.kind === "stale") return;
      // None: another session listed nothing of this one's.
      setList(rows.value && { session, rows: rows.value });
      if (rows.value) setKeptOriginalCount(rows.value.length);
    } catch (error) {
      // The last list stays; its rows carry the session they were listed in.
      if (owner()) reportUiFailure("kept-originals", error);
    }
  };
  createEffect(on(hostScope, () => void relist()));
  const open = (row: KeptOriginal) => {
    if (!bindingCurrent(props.binding) || row.name === null) return void relist();
    closeUnsavedRecovery();
    mainPaneRouter.openFile(row.path, row.name, row.kind, { inPlace: true });
  };
  return <Show when={list()?.rows.length || restored().length}>
    <section class="unsaved-recovery-entry">
      <h3>{t("unsaved-recovery-kept-heading")}</h3>
      <p>{t("unsaved-recovery-kept-detail")}</p>
      <For each={restored()}>{({ row, other }) => <div class="recovery-kept-row">
        <span>{other ? t("unsaved-recovery-restore-superseded", { path: row.path }) : t("unsaved-recovery-restored", { path: row.path })}</span>
        <button class="settings-btn" type="button" onClick={() => open(row)}>{t("unsaved-recovery-open-file")}</button>
      </div>}</For>
      <For each={list()?.rows ?? []}>{(row) => <KeptRow row={row} session={list()!.session} binding={props.binding}
        done={(outcome) => {
          if (outcome.kind === "restored" || outcome.kind === "superseded") {
            setRestored((rows) => [...rows, { row, other: outcome.kind === "superseded" }]);
          }
          void relist();
        }} />}</For>
    </section>
  </Show>;
}

function KeptRow(props: { row: KeptOriginal; session: number; binding: Binding; done: (outcome: RestoreOutcome) => void }): JSX.Element {
  const [restoring, setRestoring] = createSignal(false);
  const [message, setMessage] = createSignal("");
  const { key, origin, path } = props.row;
  const live = () => bindingCurrent(props.binding) && hostSession() === props.session;
  const restore = async () => {
    if (restoring()) return;
    setRestoring(true);
    const owner = bindingOwner(live);
    let outcome: RestoreOutcome;
    try {
      const answer = await writeOwned(owner, backend().pageRestoreKept(props.session, key, origin));
      setRestoring(false);
      if (answer.kind === "stale") return;
      outcome = answer.value;
    } catch {
      // The command did not answer: nothing is known to be written.
      setRestoring(false);
      if (!owner()) return;
      outcome = { kind: "refused", reason: "unavailable" };
    }
    reportRestore(outcome, path);
    props.done(outcome);
  };
  const copy = async () => {
    const owner = bindingOwner(live);
    try {
      const text = await readOwned(owner, backend().pageKeptText(props.session, key, origin));
      if (text.kind === "stale") return;
      if (text.value === null) throw new Error("not kept");
      await writeClipboardText(text.value);
      setMessage(t("unsaved-recovery-original-copied"));
    } catch {
      if (owner()) setMessage(t("unsaved-recovery-original-copy-failed"));
    }
  };
  return <div class="recovery-kept-row">
    <span>{path}</span>
    <button class="settings-btn" type="button" disabled={restoring()} onClick={() => void restore()}>
      {restoring() ? t("unsaved-recovery-restoring") : t("unsaved-recovery-restore")}
    </button>
    <button class="settings-btn" type="button" onClick={() => void copy()}>{t("unsaved-recovery-copy-original")}</button>
    <p role="status">{message()}</p>
  </div>;
}

export function UnsavedRecovery(): JSX.Element {
  return <Show when={unsavedRecoveryOpen()}><RecoveryPanel /></Show>;
}

function RecoveryPanel(): JSX.Element {
  const binding = captureBinding();
  const [pages, setPages] = createSignal(unsavedDrafts());
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  let root: HTMLDivElement | undefined;
  // The page client's state is not one signal; poll while the panel is open.
  const refresh = () => {
    const next = unsavedDrafts();
    if (JSON.stringify(next) !== JSON.stringify(pages())) setPages(next);
  };
  onMount(() => {
    const timer = window.setInterval(refresh, 500);
    const unregister = registerTransientLayer({ id: "unsaved-recovery", root: () => root ?? null,
      dismiss: () => { closeUnsavedRecovery(); return true; } });
    root?.focus();
    onCleanup(() => { clearInterval(timer); unregister(); });
  });
  const retry = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      const saved = await flushAll();
      setMessage(saved ? "All pending changes saved. You can close the window now." : "Some changes still need attention. Review the pages below.");
    } catch {
      setMessage("Saving failed. Your drafts remain available below.");
    } finally {
      setBusy(false);
      refresh();
    }
  };
  return <div class="modal-overlay">
    <div ref={root} tabIndex={-1} class="unsaved-recovery-panel" role="dialog" aria-modal="true" aria-label={t("unsaved-recovery-unsaved-changes")}>
      <h2>{t("unsaved-recovery-unsaved-changes")}</h2>
      <p>{t("unsaved-recovery-summary", { count: pages().length })}</p>
      <DialogActions
        cancel={<button class="settings-btn" type="button" onClick={closeUnsavedRecovery}>{t("unsaved-recovery-keep-working")}</button>}
        primary={<button class="settings-btn" type="button" disabled={busy()} onClick={() => void retry()}>{busy() ? t("unsaved-recovery-saving") : t("unsaved-recovery-retry-saving")}</button>} />
      <p role="status">{message()}</p>
      <KeptOriginals binding={binding} />
      <Show when={pages().length === 0}><p>{t("unsaved-recovery-no-pending-page-drafts-if-closing-still-fails")}</p></Show>
      <For each={pages()}>{(entry) => <section class="unsaved-recovery-entry">
        <h3>{entry.name} — {entry.state}</h3>
        <button class="settings-btn" type="button" onClick={() => {
          const kind = entry.page?.kind ?? "page";
          closeUnsavedRecovery();
          if (entry.path) mainPaneRouter.openFile(entry.path, entry.name, kind, { inPlace: true });
          else mainPaneRouter.openPage(entry.name, kind, { inPlace: true });
        }}>{entry.state === "Conflict" ? t("unsaved-recovery-open-page-to-resolve-the-conflict") : t("unsaved-recovery-open-page")}</button>
        <Show when={entry.page} fallback={<p>{t("unsaved-recovery-no-page-draft-is-available-in-this-window")}</p>}>
          {(page) => <RecoveryDraft page={page()} />}
        </Show>
      </section>}</For>
    </div>
  </div>;
}
