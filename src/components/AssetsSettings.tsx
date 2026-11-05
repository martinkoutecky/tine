import { settingLabel, settingAttributes, settingHint } from "./settingsField";
import { settings } from "./settingDefinitions";
import { For, Show, createSignal, onCleanup, type JSX } from "solid-js";
import { Rich } from "../i18n/Rich";
import { msg, t } from "../i18n";
import { backend } from "../backend";
import { flushAll } from "../document";
import { captureBinding, graphScopedSignal, refuseStaleWrite } from "../binding";
import { bindingOwner, graphOwner, latestOwner, readOwned, writeOwned } from "../owned";
import { reportUiFailure } from "../uiFailure";
import { pushToast } from "../toasts";
import type { AssetInfo, TrashStats } from "../types";

/** One graph's orphan scan (I-20): null once its binding is stale, so Trash cannot carry an old graph's name into the new one. */
const [orphanScan, setOrphanScan] = graphScopedSignal<AssetInfo[]>();

export function AssetsTab(): JSX.Element {
  let alive = true; onCleanup(() => { alive = false; setOrphanScan(null); });
  const requests = {}; const list = () => orphanScan() ?? [];
  const [busy, setBusy] = createSignal(false);
  const scanned = () => orphanScan() !== null;
  const [trashInfo, setTrashInfo] = createSignal<TrashStats>({ count: 0, bytes: 0, pages: 0, journals: 0, conflicts: 0, other: 0 });

  const fmtSize = (n: number) => t("settings-prose-file-size", n >= 1 << 20 ? { value: (n / (1 << 20)).toFixed(1), unit: "MB" } : n >= 1024 ? { value: String(Math.round(n / 1024)), unit: "KB" } : { value: String(n), unit: "B" });
  const fmtDate = (secs: number | null) => secs == null ? "" : new Date(secs * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
  const total = () => list().reduce((s, a) => s + a.size, 0);
  const protectedTrashCount = () =>
    trashInfo().pages + trashInfo().journals + trashInfo().conflicts + trashInfo().other;
  const protectedTrashArgs = () => {
    const { pages, journals, conflicts, other } = trashInfo();
    return { pages, journals, conflicts, other, beforeJournals: pages ? 1 : 0, beforeConflicts: pages || journals ? 1 : 0, beforeOther: pages || journals || conflicts ? 1 : 0 };
  };

  const refreshTrash = async () => {
    const owner = latestOwner(requests, "trash-stats", graphOwner(() => alive));
    try {
      const info = await readOwned(owner, backend().assetTrashStats());
      if (info.kind === "current") setTrashInfo(info.value);
    } catch (error) { if (owner()) reportUiFailure("trash-inventory", error); }
  };

  const refresh = async () => {
    const owner = latestOwner(requests, "scan", graphOwner(() => alive));
    setBusy(true);
    try {
      // Persist edits first so a just-deleted block's media counts as orphaned
      // (and a just-inserted one counts as referenced).
      if (!(await flushAll())) {
        if (!owner()) return;
        pushToast(msg("settings-ui-some-pages-couldn-t-be-saved-resolve-conflicts-before"), "error");
        return;
      }
      const assets = await readOwned(owner, backend().listOrphanAssets());
      if (assets.kind === "stale") return;
      setOrphanScan(assets.value);
      await refreshTrash();
    } catch (e) {
      if (owner()) reportUiFailure("asset-inventory", e);
    } finally {
      if (owner()) setBusy(false);
    }
  };

  const open = async (a: AssetInfo) => {
    const binding = captureBinding();
    const owner = graphOwner();
    try {
      await readOwned(owner, backend().openAsset(a.name, binding.backendGeneration));
    } catch (e) {
      pushToast(msg("settings-ui-couldn-t-open", { name: a.name, value1: String(e) }), "error");
    }
  };
  const trash = async (a: AssetInfo) => {
    if (orphanScan() === null) return refuseStaleWrite("Moving that asset to the trash");
    const binding = captureBinding();
    const owner = bindingOwner();
    // Recoverable trash moves directly; emptying trash still asks.
    try {
      const result = await writeOwned(owner, backend().trashAsset(a.name, binding.backendGeneration));
      if (result.kind === "stale") return;
      if (result.value === "referenced") {
        // A page started using the file after the scan (typed outcome, GH #623): keep it.
        if (alive) setOrphanScan(list().filter((x) => x.name !== a.name));
        pushToast(msg("settings-ui-is-used-by-a-page-now-so-it-was", { name: a.name }), "info");
        return;
      }
      if (alive) setOrphanScan(list().filter((x) => x.name !== a.name));
      pushToast(msg("settings-ui-moved-to-trash", { name: a.name }), "success");
      await refreshTrash();
    } catch (e) {
      pushToast(msg("settings-ui-couldn-t-trash", { value1: String(e) }), "error");
    }
  };
  const emptyTrash = async () => {
    const binding = captureBinding();
    const owner = bindingOwner();
    const info = trashInfo();
    if (!info.count) return;
    const confirmed = await readOwned(owner, backend().confirm(
        t("settings-asset-trash-confirmation", { count: info.count, size: fmtSize(info.bytes) }),
        { okLabel: t("confirm-empty-trash"), cancelLabel: t("confirm-cancel") },
      ));
    if (confirmed.kind === "stale" || !confirmed.value) return;
    try {
      const result = await writeOwned(owner, backend().emptyAssetTrash(binding.backendGeneration));
      if (result.kind === "stale") return;
      const n = result.value;
      setTrashInfo((t) => ({ ...t, count: 0, bytes: 0 }));
      pushToast(msg("settings-asset-trash-emptied", { count: n }), "success");
    } catch (e) {
      pushToast(msg("settings-ui-couldn-t-empty-trash", { value1: String(e) }), "error");
    }
  };
  return (
    <>
      <div class="settings-section" {...settingAttributes(settings["settings-orphaned-media"])}>
        {settingLabel(settings["settings-orphaned-media"])}
        <button class="settings-btn" style={{ "margin-left": "10px" }} disabled={busy()} onClick={() => void refresh()}>
          {busy() ? t("settings-scanning") : scanned() ? t("settings-rescan") : t("settings-scan-for-orphans")}
        </button>
        <Show when={trashInfo().count > 0}>
          <button
            class="settings-btn settings-btn-danger"
            style={{ "margin-left": "8px" }}
            onClick={() => void emptyTrash()}
            title={t("settings-ui-permanently-delete-asset-files-in-logseq-tine-trash")}
          >{t("settings-ui-empty-asset-trash", { count: trashInfo().count })}</button>
        </Show>
      </div>
      <Show when={protectedTrashCount() > 0}>
        <div class="settings-hint settings-block">{t("settings-trash-protected-summary", protectedTrashArgs())}</div>
      </Show>
      <div class="settings-hint settings-block"><Rich message={settingHint(settings["settings-orphaned-media"])} /></div>
      <Show when={scanned()}>
        <Show
          when={list().length}
          fallback={<div class="settings-hint settings-block">{t("settings-ui-no-orphaned-media-every-asset-is-referenced")}</div>}
        >
          <div class="settings-hint settings-block">{t("settings-orphaned-assets-summary", { count: list().length, size: fmtSize(total()) })}</div>
          <div class="settings-backups">
            <For each={list()}>
              {(a) => (
                <div class="settings-asset-row">
                  <button class="settings-asset-name mono" title={t("settings-ui-open-in-the-default-app")} onClick={() => void open(a)}>
                    {a.name}
                  </button>
                  <span class="settings-asset-date">{fmtDate(a.modified)}</span>
                  <span class="settings-backup-files mono">{fmtSize(a.size)}</span>
                  <button class="settings-btn" onClick={() => void trash(a)}>{t("settings-ui-trash")}</button>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </>
  );
}
