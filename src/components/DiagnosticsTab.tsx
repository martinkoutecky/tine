// Settings → Help & diagnostics: review, copy, save (desktop) or clear the
// privacy-safe diagnostic report of this run and the previous one (GH #343,
// og ADR 0058), verify a synchronized graph's exact bytes against another
// device, list this session's recent error messages (memory only), and run the parser comparison
// ("Help improve Tine's parser"). Nothing here is uploaded automatically.
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
import { ImproveTab } from "./ImproveTab";
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
      pushToast("Could not create the diagnostic report.", "error");
    } finally {
      if (!disposed) setBusy(false);
    }
  };

  // One full stat diff of the open graph on demand (the same path as the
  // rescan on return to the window, without its throttle). Shows when it ended.
  const rescanGraph = async () => {
    setRescanning(true);
    try {
      const finished = await rescanGraphNowFromSettings();
      if (!disposed) setRescanFinished(finished);
      if (finished === null && !disposed) pushToast("No graph rescan ran. Open a graph first.", "info");
    } catch (error) {
      dbg(`graph rescan failed: ${String(error)}`);
      pushToast("Could not rescan the graph.", "error");
    } finally {
      if (!disposed) setRescanning(false);
    }
  };

  const copyReport = async () => {
    const current = report();
    if (!current) return;
    try {
      await writeClipboardText(current.text);
      pushToast("Diagnostic report copied", "success");
    } catch (error) {
      dbg(`diagnostic report copy failed: ${String(error)}`);
      pushToast("Could not copy the diagnostic report.", "error");
    }
  };

  const saveReport = async () => {
    try {
      const saved = await writeOwned(ownedWhen(() => !disposed), backend().saveDiagnosticReport(__GIT_COMMIT__, __BUILD_TIME__));
      if (saved.kind === "current" && saved.value) pushToast("Diagnostic report saved", "success");
    } catch (error) {
      dbg(`diagnostic report save failed: ${String(error)}`);
      pushToast("Could not save the diagnostic report.", "error");
    }
  };

  const clearReport = async () => {
    try {
      const result = await writeOwned(ownedWhen(() => !disposed), backend().clearDiagnostics());
      if (result.kind === "current") setReport(null);
      pushToast("Recorded diagnostic events cleared", "success");
    } catch (error) {
      dbg(`diagnostic clear failed: ${String(error)}`);
      pushToast("Could not clear the recorded diagnostic events.", "error");
    }
  };

  return (
    <section class="diagnostics-tab settings-section">
      <h2>Help & diagnostics</h2>
      <p>
        Tine keeps a small, bounded flight recorder of this run and the previous one, including
        whether the previous run closed cleanly. It records operation names, outcomes, timings,
        counts, platform and build information.
      </p>
      <p class="settings-hint diagnostics-privacy">
        It does not record graph content, file paths, page titles, queries, URLs, credentials, or
        the detailed opt-in debug log. It is kept in Tine's private app data (at most 1 MiB), never
        in your graph, and nothing is uploaded. You choose whether to copy or save a report and
        share it.
      </p>
      <p class="settings-hint diagnostics-privacy">
        A report also carries statistics about your graph, such as how long each step of opening
        it took and the sizes of its pages, as numbers only. They never include a page name, any
        text, or a hash of either.
      </p>
      <div class="diagnostics-actions">
        <button type="button" class="primary" disabled={busy()} onClick={() => void createReport()}>
          {busy() ? "Creating…" : "Create diagnostic report"}
        </button>
        <Show when={report()}>
          <button type="button" onClick={() => void copyReport()}>Copy report</button>
        </Show>
        <Show when={!isMobilePlatform}>
          <button type="button" onClick={() => void saveReport()}>Save report…</button>
        </Show>
        <button type="button" class="danger" onClick={() => void clearReport()}>
          Clear recorded events
        </button>
      </div>
      <div class="diagnostics-rescan">
        <button type="button" disabled={rescanning()} onClick={() => void rescanGraph()}>
          {rescanning() ? "Rescanning…" : "Rescan graph"}
        </button>
        <span class="settings-hint" role="status">
          <Show
            when={rescanFinished()}
            fallback="Checks every file in the open graph for changes made outside Tine."
          >
            {(finished) => `Last rescan finished at ${new Date(finished()).toLocaleTimeString()}.`}
          </Show>
        </span>
      </div>
      <Show when={report()}>
        {(current) => (
          <label class="diagnostics-preview">
            <span>Report preview · {current().suggestedFileName}</span>
            <Show when={current().text.length > DIAGNOSTIC_PREVIEW_LIMIT}>
              <span class="settings-hint">
                Large report: this preview is shortened to keep Settings responsive. Copy report
                exports the complete report.
              </span>
            </Show>
            <textarea readonly spellcheck={false} value={diagnosticReportPreview(current().text)} />
          </label>
        )}
      </Show>
      <ErrorToastHistory />
      <GraphVerification />
      <div class="diagnostics-improve">
        <h3>Help improve Tine's parser</h3>
        <ImproveTab />
      </div>
    </section>
  );
}
