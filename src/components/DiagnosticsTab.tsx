import { msg, t } from "../i18n";
// Settings → Help & diagnostics: review, copy, save (desktop) or clear the
// privacy-safe diagnostic report of this run and the previous one (GH #343,
// og ADR 0058), verify a synchronized graph's exact bytes against another
// device, list this session's recent error messages (memory only). Nothing here is uploaded automatically.
import { Show, createSignal, onCleanup, type JSX } from "solid-js";
import { backend, type DiagnosticReport } from "../backend";
import { writeClipboardText } from "../clipboard";
import { dbg } from "../debug";
import { isMobilePlatform } from "../nativeChrome";
import { ownedWhen, readOwned, writeOwned } from "../owned";
import { rescanGraphNowFromSettings } from "../reloadOnFocus";
import { pushToast } from "../toasts";
import { ErrorToastHistory } from "./ErrorToastHistory";
import { GraphVerification } from "./GraphVerification";
import "../styles/diagnostics.css";

export const DIAGNOSTIC_PREVIEW_LIMIT = 64 * 1024;
const DIAGNOSTIC_PREVIEW_TAIL = 8 * 1024;

/** The on-screen review text: the whole report up to 64 KiB, otherwise its
 * head and last 8 KiB around a notice naming how much was left out. Keeps the
 * selectable WebView control responsive on Windows; Copy report still uses
 * the complete `DiagnosticReport.text`. Pure; O(report length). */
export function diagnosticReportPreview(text: string): string {
  if (text.length <= DIAGNOSTIC_PREVIEW_LIMIT) return text;
  const headLength = DIAGNOSTIC_PREVIEW_LIMIT - DIAGNOSTIC_PREVIEW_TAIL;
  const omitted = text.length - DIAGNOSTIC_PREVIEW_LIMIT;
  return `${text.slice(0, headLength)}\n\n[Preview shortened: ${omitted} characters omitted. Copy report exports the complete report.]\n\n${text.slice(-DIAGNOSTIC_PREVIEW_TAIL)}`;
}

export function DiagnosticsTab(): JSX.Element {
  const [report, setReport] = createSignal<DiagnosticReport | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [rescanning, setRescanning] = createSignal(false);
  const [rescanFinished, setRescanFinished] = createSignal<number | null>(null);
  let disposed = false;
  onCleanup(() => { disposed = true; });

  const createReport = async () => {
    setBusy(true);
    try {
      const result = await readOwned(ownedWhen(() => !disposed), backend().diagnosticReport(__GIT_COMMIT__, __BUILD_TIME__));
      if (result.kind === "current") setReport(result.value);
    } catch (error) {
      dbg(`diagnostic report failed: ${String(error)}`);
      pushToast(msg("diagnostics-tab-could-not-create-the-diagnostic-report"), "error");
    } finally {
      if (!disposed) setBusy(false);
    }
  };

  // A forced full rebuild of the open graph on demand: every file re-read and
  // re-parsed, ignoring stamps (the rescan on return to the window stays the
  // cheap stat diff). Shows when it ended.
  const rescanGraph = async () => {
    setRescanning(true);
    try {
      const finished = await rescanGraphNowFromSettings();
      if (!disposed) setRescanFinished(finished);
      if (finished === null && !disposed) pushToast(msg("diagnostics-tab-no-graph-rescan-ran-open-a-graph-first"), "info");
    } catch (error) {
      dbg(`graph rescan failed: ${String(error)}`);
      pushToast(msg("diagnostics-tab-could-not-rescan-the-graph"), "error");
    } finally {
      if (!disposed) setRescanning(false);
    }
  };

  const copyReport = async () => {
    const current = report();
    if (!current) return;
    try {
      await writeClipboardText(current.text);
      pushToast(msg("diagnostics-tab-diagnostic-report-copied"), "success");
    } catch (error) {
      dbg(`diagnostic report copy failed: ${String(error)}`);
      pushToast(msg("diagnostics-tab-could-not-copy-the-diagnostic-report"), "error");
    }
  };

  const saveReport = async () => {
    try {
      const saved = await writeOwned(ownedWhen(() => !disposed), backend().saveDiagnosticReport(__GIT_COMMIT__, __BUILD_TIME__));
      if (saved.kind === "current" && saved.value) pushToast(msg("diagnostics-tab-diagnostic-report-saved"), "success");
    } catch (error) {
      dbg(`diagnostic report save failed: ${String(error)}`);
      pushToast(msg("diagnostics-tab-could-not-save-the-diagnostic-report"), "error");
    }
  };

  const clearReport = async () => {
    try {
      const result = await writeOwned(ownedWhen(() => !disposed), backend().clearDiagnostics());
      if (result.kind === "current") setReport(null);
      pushToast(msg("diagnostics-tab-recorded-diagnostic-events-cleared"), "success");
    } catch (error) {
      dbg(`diagnostic clear failed: ${String(error)}`);
      pushToast(msg("diagnostics-tab-could-not-clear-the-recorded-diagnostic-events"), "error");
    }
  };

  return (
    <section class="diagnostics-tab settings-section">
      <h2>{t("diagnostics-tab-help-diagnostics")}</h2>
      <p>{t("diagnostics-tab-tine-keeps-a-small-bounded-flight-recorder-of")}</p>
      <p class="settings-hint diagnostics-privacy">{t("diagnostics-tab-it-does-not-record-graph-content-file-paths")}</p>
      <p class="settings-hint diagnostics-privacy">{t("diagnostics-tab-a-report-also-carries-statistics-about-your-graph")}</p>
      <div class="diagnostics-actions">
        <button type="button" class="settings-btn settings-btn-primary" disabled={busy()} onClick={() => void createReport()}>
          {busy() ? t("diagnostics-tab-creating") : t("diagnostics-tab-create-diagnostic-report")}
        </button>
        <Show when={report()}>
          <button class="settings-btn" type="button" onClick={() => void copyReport()}>{t("diagnostics-tab-copy-report")}</button>
        </Show>
        <Show when={!isMobilePlatform}>
          <button class="settings-btn" type="button" onClick={() => void saveReport()}>{t("diagnostics-tab-save-report")}</button>
        </Show>
        <button type="button" class="settings-btn danger" onClick={() => void clearReport()}>{t("diagnostics-tab-clear-recorded-events")}</button>
      </div>
      <div class="diagnostics-rescan">
        <button class="settings-btn" type="button" disabled={rescanning()} onClick={() => void rescanGraph()}>
          {rescanning() ? t("diagnostics-tab-rescanning") : t("diagnostics-tab-rescan-graph")}
        </button>
        <span class="settings-hint" role="status">
          <Show
            when={rescanFinished()}
            fallback={t("settings-prose-re-reads-every-file-in-the-open-graph-and-rebuilds")}
          >
            {(finished) => t("settings-prose-last-rescan", { time: new Date(finished()).toLocaleTimeString() })}
          </Show>
        </span>
      </div>
      <Show when={report()}>
        {(current) => (
          <label class="diagnostics-preview">
            <span>{t("diagnostics-tab-preview-file", { file: current().suggestedFileName })}</span>
            <Show when={current().text.length > DIAGNOSTIC_PREVIEW_LIMIT}>
              <span class="settings-hint">{t("diagnostics-tab-large-report-this-preview-is-shortened-to-keep")}</span>
            </Show>
            <textarea readonly spellcheck={false} value={diagnosticReportPreview(current().text)} />
          </label>
        )}
      </Show>
      <ErrorToastHistory />
      <GraphVerification />
    </section>
  );
}
