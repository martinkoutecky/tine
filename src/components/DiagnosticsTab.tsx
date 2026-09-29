// Settings → Help & diagnostics: review, copy or clear the privacy-safe
// diagnostic report of this run (GH #343), and run the parser comparison
// ("Help improve Tine's parser"). Nothing here is uploaded automatically.
import { Show, createSignal, onCleanup, type JSX } from "solid-js";
import { backend, type DiagnosticReport } from "../backend";
import { writeClipboardText } from "../clipboard";
import { dbg } from "../debug";
import { ownedWhen, readOwned, writeOwned } from "../owned";
import { pushToast } from "../toasts";
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
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const owner = () => ownedWhen(() => !disposed);

  const createReport = async () => {
    setBusy(true);
    try {
      const result = await readOwned(owner(), backend().diagnosticReport(__GIT_COMMIT__, __BUILD_TIME__));
      if (result.kind === "current") setReport(result.value);
    } catch (error) {
      dbg(`diagnostic report failed: ${String(error)}`);
      pushToast("Could not create the diagnostic report.", "error");
    } finally {
      if (!disposed) setBusy(false);
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

  const clearReport = async () => {
    try {
      const result = await writeOwned(owner(), backend().clearDiagnostics());
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
        Tine keeps a small, bounded flight recorder for the current run. It records operation
        names, outcomes, timings, counts, platform and build information.
      </p>
      <p class="settings-hint diagnostics-privacy">
        It does not record graph content, file paths, page titles, queries, URLs, credentials, or
        the detailed opt-in debug log. Nothing is uploaded automatically, and nothing is kept after
        Tine quits. You choose whether to copy a report and share it.
      </p>
      <div class="diagnostics-actions">
        <button type="button" class="primary" disabled={busy()} onClick={() => void createReport()}>
          {busy() ? "Creating…" : "Create diagnostic report"}
        </button>
        <Show when={report()}>
          <button type="button" onClick={() => void copyReport()}>Copy report</button>
        </Show>
        <button type="button" class="danger" onClick={() => void clearReport()}>
          Clear recorded events
        </button>
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
      <div class="diagnostics-improve">
        <h3>Help improve Tine's parser</h3>
        <ImproveTab />
      </div>
    </section>
  );
}
