import { Show, createSignal, type JSX } from "solid-js";
import { backend } from "../backend";
import { exportSheets } from "../sheet/exportSheets";
import { graphMeta } from "../graphSession";
import { switchGraph } from "../graph";
import { graphOwner, readOwned, writeOwned } from "../owned";

/** The Graph settings publication control. A picked external folder receives
 * one create-only site; the app snapshot and static HTML contain only the
 * selected public pages unless the user explicitly includes every page. */
export function GraphPublish(): JSX.Element {
  const [name, setName] = createSignal("Tine graph");
  const [allPages, setAllPages] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  const publish = async () => {
    const owner = graphOwner();
    const selected = await readOwned(owner, backend().pickFolder("Choose a folder outside this graph for the export"));
    if (selected.kind !== "current" || !selected.value) return;
    const destination = selected.value;
    setBusy(true);
    setMessage("Exporting…");
    try {
      const result = await writeOwned(owner, backend().publishLive(destination, name().trim() || "Tine graph", allPages(), await exportSheets(undefined, { kind: "live", allPages: allPages() })));
      if (result.kind === "current") setMessage(`Exported ${result.value.pages} pages to ${result.value.path}`);
    } catch (error) {
      if (owner()) setMessage(`Export failed: ${String((error as Error)?.message ?? error)}`);
    } finally { if (owner()) setBusy(false); }
  };
  return <>
    <div class="settings-row"><span class="settings-label">Graph</span><div>
      <span class="settings-value mono">{graphMeta()?.root ?? "—"}</span>
      <div style={{ "margin-top": "6px" }}><button class="settings-btn" onClick={() => void switchGraph()}>Open another graph…</button></div>
    </div></div>
    <div class="settings-row"><span class="settings-label">Publish</span><div>
      <label class="settings-hint">Export name <input value={name()} onInput={(event) => setName(event.currentTarget.value)} /></label>
      <label class="settings-hint"><input type="checkbox" checked={allPages()}
        onChange={(event) => setAllPages(event.currentTarget.checked)} /> Include every page not marked public:: false</label>
      <div><button class="settings-btn" disabled={busy()} onClick={() => void publish()}>Export HTML and read-only app…</button></div>
      <div class="settings-hint">Choose a destination outside the graph. The export stays on your device until you share it.</div>
      <Show when={message()}><div class="settings-hint" role="status">{message()}</div></Show>
    </div></div>
  </>;
}
