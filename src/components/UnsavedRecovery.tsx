import { msg, t } from "../i18n";
import { DialogActions } from "./DialogActions";
// The unsaved-changes recovery panel (GH #540, master dc3f2104b). Opened when the
// user declines to discard at close, or from a failed-save toast. It lists every
// page whose edits are not on disk and offers the ways out that never lose text:
// retry the ordinary save, open the page (its conflict bar resolves it), or copy
// the draft. Copying is read-only and never acknowledges a save.
import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { flushAll, restoreSource, unsavedDrafts, type RestorableSource } from "../document";
import { keptSources, restorableSources } from "../restorableSources";
import { pushToast } from "../toasts";
import { mainPaneRouter } from "../router";
import { writeClipboardText } from "../clipboard";
import { closeUnsavedRecovery, unsavedRecoveryOpen } from "../unsavedRecovery";
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

/** ` (<platform step>, os error <n>)` of a failed step, when the host named
 * them (as a failed save's toast says it, GH #538). */
function failedStep(refusal: { operation: string | null; osError: number | null }): string {
  const parts = [refusal.operation, refusal.osError === null ? null : `os error ${refusal.osError}`]
    .filter((part): part is string => !!part);
  return parts.length ? ` (${parts.join(", ")})` : "";
}

/** What the host's record of this source's latest restore says (`RestoreState`),
 * the one answer to "what happened to this restore"; nothing before any. */
function restoreStatus(source: RestorableSource): string {
  const state = source.state;
  const path = source.path;
  if (!state) return "";
  if (state.kind === "restored") return t("unsaved-recovery-restored", { path });
  if (state.kind === "restoring") {
    const problem = state.problem;
    if (!problem) return t("unsaved-recovery-restoring");
    if (problem.reason === "claimed") return t("unsaved-recovery-restore-retrying-claimed", { path, file: problem.file });
    if (problem.reason === "failed") return t("unsaved-recovery-restore-retrying-failed", { path, step: failedStep(problem) });
    return t("unsaved-recovery-restoring");
  }
  switch (state.reason) {
    case "occupied": return t("unsaved-recovery-restore-refused-occupied", { path });
    case "claimed": return t("unsaved-recovery-restore-refused-claimed", { path, file: state.file });
    case "unsaved": return t("unsaved-recovery-restore-refused-unsaved", { path });
    case "failed": return t("unsaved-recovery-restore-refused-failed", { path, step: failedStep(state) });
  }
}

/** A rename's source whose original text a cancelled rename kept (the host's
 * restorable list): Restore puts the text back through the page's own guarded
 * save. How it stands is the host's record, which the list carries (restoring,
 * retrying and why, refused and why, restored); the command's reply only says
 * whether the host recorded it. The text stays here to copy until restored. */
function KeptSource(props: { source: RestorableSource }): JSX.Element {
  const [message, setMessage] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const path = () => props.source.path;
  const kind = () => props.source.state?.kind;
  const copy = async () => {
    try {
      await writeClipboardText(props.source.text);
      setMessage(t("unsaved-recovery-original-copied"));
    } catch {
      setMessage(t("unsaved-recovery-original-copy-failed"));
    }
  };
  const restore = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      const outcome = await restoreSource(props.source.key);
      if (outcome === "waiting") pushToast(msg("unsaved-recovery-restore-waiting", { path: path() }), "info");
      else if (outcome === "uncertain") pushToast(msg("unsaved-recovery-restore-uncertain", { path: path() }), "info");
      else if (outcome === "refused") pushToast(msg("unsaved-recovery-restore-refused", { path: path() }), "error");
    } catch (error) {
      pushToast(msg("unsaved-recovery-restore-failed", { path: path(), error: String(error) }), "error");
    } finally {
      setBusy(false);
    }
  };
  return <section class="unsaved-recovery-entry">
    <Show when={kind() !== "restored"} fallback={<p role="status">{restoreStatus(props.source)}</p>}>
      <h3>{t("unsaved-recovery-kept-after-a-cancelled-rename", { path: path() })}</h3>
      <p>{t("unsaved-recovery-kept-after-a-cancelled-rename-detail", { path: path() })}</p>
      <Show when={kind() !== "restoring"}>
        <button class="settings-btn" type="button" disabled={busy()} onClick={() => void restore()}>{t("unsaved-recovery-restore")}</button>
      </Show>
      <p role="status">{restoreStatus(props.source)}</p>
      <div class="recovery-draft">
        <pre tabIndex={0}>{props.source.text}</pre>
        <button class="settings-btn" type="button" onClick={() => void copy()}>{t("unsaved-recovery-copy-draft")}</button>
        <p role="status">{message()}</p>
      </div>
    </Show>
  </section>;
}

export function UnsavedRecovery(): JSX.Element {
  return <Show when={unsavedRecoveryOpen()}><RecoveryPanel /></Show>;
}

function RecoveryPanel(): JSX.Element {
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
      <For each={restorableSources()}>{(source) => <KeptSource source={source} />}</For>
      <Show when={pages().length === 0 && keptSources().length === 0}><p>{t("unsaved-recovery-no-pending-page-drafts-if-closing-still-fails")}</p></Show>
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
