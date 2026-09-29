// Settings → Help & diagnostics → "Verify synchronized graph": fingerprint the
// exact bytes of this device's Markdown and Org files and compare them with the
// report from another device, naming every path that differs (master 749bfb2b1).
// The report lists file paths, never file contents; nothing is uploaded.
import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { writeClipboardText } from "../clipboard";
import { dbg } from "../debug";
import {
  compareGraphVerificationManifests,
  parseGraphVerificationManifest,
  type GraphVerificationComparison,
  type GraphVerificationProgress,
  type GraphVerificationReport,
} from "../graphVerification";
import { isMobilePlatform } from "../nativeChrome";
import { graphOwner, readOwned, writeOwned } from "../owned";
import { pushToast } from "../toasts";

const isCancellation = (error: unknown) => String(error).toLowerCase().includes("cancelled");

export function GraphVerification(): JSX.Element {
  const [report, setReport] = createSignal<GraphVerificationReport | null>(null);
  const [progress, setProgress] = createSignal<GraphVerificationProgress | null>(null);
  const [operation, setOperation] = createSignal<string | null>(null);
  const [other, setOther] = createSignal("");
  const [comparison, setComparison] = createSignal<GraphVerificationComparison | null>(null);
  let disposed = false;
  let stopProgress: (() => void) | undefined;

  onMount(() => {
    void backend().onGraphVerificationProgress((event) => {
      if (event.operationId === operation()) setProgress(event);
    }).then((stop) => {
      if (disposed) stop();
      else stopProgress = stop;
    });
  });
  onCleanup(() => {
    disposed = true;
    stopProgress?.();
  });

  const create = async () => {
    const id = globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`;
    const owner = graphOwner(() => !disposed);
    setOperation(id);
    setProgress({ operationId: id, processed: 0, total: 0 });
    setComparison(null);
    try {
      const result = await readOwned(owner, backend().createGraphVerification(id));
      if (result.kind === "current") {
        setReport(result.value);
        if (!result.value.complete) pushToast("Graph verification was incomplete", "error");
      }
    } catch (error) {
      if (!isCancellation(error)) {
        dbg(`graph verification failed: ${String(error)}`);
        pushToast("Could not verify graph files.", "error");
      }
    } finally {
      if (!disposed) setOperation(null);
    }
  };

  const cancel = async () => {
    const id = operation();
    if (!id) return;
    try {
      await backend().cancelGraphVerification(id);
    } catch (error) {
      dbg(`graph verification cancel failed: ${String(error)}`);
    }
  };

  const copy = async () => {
    const current = report();
    if (!current) return;
    try {
      await writeClipboardText(current.text);
      pushToast("Graph verification report copied", "success");
    } catch (error) {
      dbg(`graph verification copy failed: ${String(error)}`);
      pushToast("Could not copy the graph verification report.", "error");
    }
  };

  const save = async () => {
    const current = report();
    if (!current) return;
    try {
      const saved = await writeOwned(graphOwner(() => !disposed), backend().saveGraphVerificationReport(current.text));
      if (saved.kind === "current" && saved.value) pushToast("Graph verification report saved", "success");
    } catch (error) {
      dbg(`graph verification save failed: ${String(error)}`);
      pushToast("Could not save the graph verification report.", "error");
    }
  };

  const compare = () => {
    const current = report();
    if (!current) return;
    try {
      setComparison(compareGraphVerificationManifests(
        parseGraphVerificationManifest(current.text),
        parseGraphVerificationManifest(other()),
      ));
    } catch (error) {
      setComparison(null);
      pushToast(`Could not compare the reports: ${error instanceof Error ? error.message : "invalid report."}`, "error");
    }
  };

  return (
    <div class="diagnostics-verification">
      <h3>Verify synchronized graph</h3>
      <p>
        Compare the exact Markdown and Org file bytes on two devices. The report includes file
        paths and page names, but not file contents. Nothing is uploaded automatically.
      </p>
      <div class="diagnostics-actions">
        <button type="button" class="primary" disabled={operation() !== null} onClick={() => void create()}>
          {operation() ? "Verifying…" : "Create graph verification report"}
        </button>
        <Show when={operation()}>
          <button type="button" onClick={() => void cancel()}>Cancel</button>
        </Show>
        <Show when={report()}>
          <button type="button" onClick={() => void copy()}>Copy graph report</button>
          <Show when={!isMobilePlatform}>
            <button type="button" onClick={() => void save()}>Save graph report…</button>
          </Show>
        </Show>
      </div>
      <Show when={operation() !== null ? progress() : null}>
        {(current) => (
          <p class="settings-hint">
            {current().total === 0 ? "Reading graph file list…" : `${current().processed} / ${current().total} files`}
          </p>
        )}
      </Show>
      <Show when={report()}>
        {(current) => (
          <>
            <p class="settings-hint">
              {current().complete ? "Complete" : "Incomplete"} · {current().totalFiles} files · {current().totalBytes} bytes
            </p>
            <label class="diagnostics-preview">
              <span>Report preview · {current().suggestedFileName}</span>
              <textarea readonly spellcheck={false} value={current().text} />
            </label>
            <label class="diagnostics-preview">
              <span>Report from the other device</span>
              <textarea
                spellcheck={false}
                value={other()}
                onInput={(event) => setOther(event.currentTarget.value)}
                placeholder="Paste the graph verification report here"
              />
            </label>
            <button type="button" class="primary" disabled={!other().trim()} onClick={compare}>
              Compare reports
            </button>
          </>
        )}
      </Show>
      <Show when={comparison()}>
        {(result) => (
          <div class="diagnostics-comparison">
            <Show when={result().matches}>
              <p><strong>The source file sets and bytes match.</strong></p>
            </Show>
            <Show when={result().incomplete}>
              <p><strong>At least one report is incomplete. No match can be confirmed.</strong></p>
            </Show>
            <For each={[
              ["Only on this device", result().localOnly],
              ["Only on the other device", result().otherOnly],
              ["Different bytes", result().changed],
            ] as const}>
              {([label, paths]) => (
                <Show when={paths.length > 0}>
                  <h4>{label}</h4>
                  <ul><For each={paths}>{(path) => <li><code>{path}</code></li>}</For></ul>
                </Show>
              )}
            </For>
          </div>
        )}
      </Show>
    </div>
  );
}
