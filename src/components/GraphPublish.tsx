import { Show, createSignal, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { exportSheets } from "../sheet/exportSheets";
import { graphMeta } from "../graphSession";
import { switchGraph } from "../graph";
import { bindingOwner, readOwned, writeOwned } from "../owned";
import { platformKind } from "../platform";
import { t } from "../i18n";
import { pushToast } from "../toasts";

/** The Graph settings publication control. A picked external folder receives
 * one create-only site; the app snapshot and static HTML contain only the
 * selected public pages unless the user explicitly includes every page. */
export function GraphPublish(): JSX.Element {
  const [name, setName] = createSignal("Tine graph");
  const [allPages, setAllPages] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal("");
  const [exportPath, setExportPath] = createSignal("");
  const [desktop, setDesktop] = createSignal(false);
  let mounted = true;
  onCleanup(() => { mounted = false; });
  const publish = async () => {
    const owner = bindingOwner(() => mounted);
    setBusy(true);
    try {
      const previous = await readOwned(owner, backend().getAppString("html_export_directory", ""));
      if (previous.kind !== "current") return;
      const selected = await readOwned(owner, backend().pickFolder("Choose a folder outside this graph for the export", previous.value || undefined));
      if (selected.kind !== "current" || !owner() || !selected.value) return;
      const destination = selected.value;
      const everyPage = allPages();
      setMessage("Exporting…");
      const exportName = name().trim() || "Tine graph";
      const sheets = await readOwned(owner, exportSheets(undefined, { kind: "live", allPages: everyPage }));
      if (sheets.kind === "stale" || !owner()) return;
      const result = await writeOwned(owner, backend().publishLive(destination, exportName, everyPage, sheets.value));
      // A zero reads as a broken button unless it names the rule (GH #560, master 350efef1f).
      if (result.kind === "current" && owner()) setMessage(result.value.pages === 0 && !everyPage
        ? `Exported 0 pages to ${result.value.path} — only pages with “public:: true” are exported.`
        : `Exported ${result.value.pages} pages to ${result.value.path}`);
      if (result.kind !== "current" || !owner()) return;
      setExportPath(result.value.path);
      try {
        await writeOwned(owner, backend().setAppString("html_export_directory", destination));
      } catch (error) { if (owner()) pushToast(t("export-remember-failed", { error: String(error) }), "error"); }
      const platform = await readOwned(owner, platformKind());
      if (platform.kind === "current") setDesktop(platform.value === "desktop");
    } catch (error) {
      if (owner()) setMessage(`Export failed: ${String((error as Error)?.message ?? error)}`);
    } finally { if (owner()) setBusy(false); }
  };
  return <>
    <div class="settings-row"><span class="settings-label">{t("graph-publish-graph")}</span><div>
      <span class="settings-value mono">{graphMeta()?.display_root ?? graphMeta()?.root ?? "—"}</span>
      <div style={{ "margin-top": "6px" }}><button class="settings-btn" onClick={() => void switchGraph()}>{t("graph-publish-open-another-graph")}</button></div>
    </div></div>
    <div class="settings-row"><span class="settings-label">{t("graph-publish-publish")}</span><div>
      <label class="settings-hint">{t("graph-publish-export-name")}<input value={name()} onInput={(event) => setName(event.currentTarget.value)} /></label>
      <label class="settings-hint"><input type="checkbox" checked={allPages()}
        onChange={(event) => setAllPages(event.currentTarget.checked)} />{t("graph-publish-include-every-page-not-marked-public-false")}</label>
      <div><button class="settings-btn" disabled={busy()} onClick={() => void publish()}>{t("graph-publish-export-html-and-read-only-app")}</button></div>
      <div class="settings-hint">{t("graph-publish-choose-a-destination-outside-the-graph-the-export")}</div>
      <Show when={message()}><div class="settings-hint" role="status">{message()}</div></Show>
      <Show when={desktop() && exportPath()}>
        <button class="settings-btn" onClick={() => {
          const owner = bindingOwner(() => mounted);
          void writeOwned(owner, backend().revealExport(exportPath())).catch((error) => pushToast(t("export-reveal-failed", { error: String(error) }), "error"));
        }}>{t("export-show-folder")}</button>
      </Show>
    </div></div>
  </>;
}
