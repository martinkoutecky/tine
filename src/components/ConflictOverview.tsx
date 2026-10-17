import { msg, t, render, type Msg } from "../i18n";
import { For, Show, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { bindingOwner, readOwned, writeOwned } from "../owned";
import type { PaneRouter } from "../router";
import { pushToast } from "../toasts";
import type { ConflictObject, ConflictSource, SyncConflict } from "../types";
import { conflictQueue, openPageInSidebar, refreshSyncConflicts, syncConflicts } from "../ui";
import { liveConflictObjects, pendingConflictCount } from "../liveConflicts";

// Concord overview (og 8c): every page that needs a decision, in one place,
// rendered from the derived conflict queue and never written to the graph. It
// is the INVENTORY; resolution happens on the page, next to the blocks. Rows
// vanish as the queue re-derives after each Apply.

const GROUPS: { source: ConflictSource; title: Msg }[] = [
  { source: "live-save", title: msg("conflict-overview-unsaved-drafts") },
  { source: "sync-copy", title: msg("conflict-overview-sync-conflict-copies") },
  { source: "vcs-markers", title: msg("conflict-overview-version-control-merge-markers") },
  { source: "duplicate-journal", title: msg("conflict-overview-duplicate-journal-days") },
];

export function conflictSourceLabel(conflict: ConflictObject): string {
  const side = (role: "mine" | "theirs") => conflict.sides.find((s) => s.role === role)?.label;
  if (conflict.source === "live-save") return t("conflicts-prose-unsaved-draft");
  if (conflict.source === "duplicate-journal") return t("conflicts-prose-day-files", { count: conflict.sides.length });
  return conflict.source === "sync-copy"
    ? t("conflicts-prose-sync-copy", { side: side("theirs") ?? t("conflicts-prose-copy-fallback") })
    : t("conflicts-prose-merge-sides", { mine: side("mine") ?? t("conflicts-prose-local-fallback"), theirs: side("theirs") ?? t("conflicts-prose-incoming-fallback") });
}

/** Absent is not zero: a count Tine did not compute shows "—", never "0". */
export function conflictBlockCount(conflict: ConflictObject): string {
  const count = conflict.block_conflicts;
  if (count === undefined || count === null) return "—";
  return t("conflicts-prose-block-count", { count });
}

/** The conflict copy's own file, for a sync-copy object. */
function copyPath(conflict: ConflictObject): string | undefined {
  return conflict.sides.find((s) => s.role === "theirs")?.path ?? undefined;
}

export function ConflictOverview(props: { router: PaneRouter }): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  onMount(() => void refreshSyncConflicts());
  const discardCopy = async (path: string, pageName: string) => {
    const owner = bindingOwner(() => alive);
    const name = path.split("/").pop() ?? path;
    const confirmed = await readOwned(owner, backend().confirm(
      t("conflict-overview-discard-confirmation", { name, page: pageName })
    ));
    if (confirmed.kind === "stale" || !confirmed.value) return;
    try {
      const result = await writeOwned(owner, backend().trashSyncConflict(path, "delete-page"));
      if (result.kind === "stale") return;
      pushToast(msg("conflict-overview-discarded", { name: String(name) }), "success");
      await readOwned(owner, refreshSyncConflicts());
    } catch (e) {
      pushToast(msg("conflict-overview-couldn-t-discard-it", { error: String(e) }), "error");
    }
  };
  const orphans = (): SyncConflict[] => syncConflicts().filter((c) => !c.base_path);
  const open = (conflict: ConflictObject, event: MouseEvent) => {
    const target = { name: conflict.page_name, pageKind: conflict.kind, path: conflict.page_path };
    // Shift- or middle-click keeps the overview in view beside the page.
    if (event.shiftKey || event.button === 1) openPageInSidebar(target);
    else props.router.openPageTarget(target);
  };
  const total = pendingConflictCount;
  return (
    <div class="conflict-overview">
      <h1 class="page-title">{t("conflict-overview-conflicts")}</h1>
      <Show
        when={total() > 0}
        fallback={<p class="conflict-overview-empty">{t("conflict-overview-no-conflicts-pages-that-need-a-decision-appear")}</p>}
      >
        <p class="conflict-overview-hint">{t("conflict-overview-each-page-is-resolved-on-the-page-itself")}</p>
        <For each={GROUPS}>
          {(group) => {
            // A live draft is not on disk, so it is not in the derived queue.
            const rows = () => group.source === "live-save" ? liveConflictObjects() : conflictQueue().filter((c) => c.source === group.source);
            const extra = () => (group.source === "sync-copy" ? orphans() : []);
            return (
              <Show when={rows().length || extra().length}>
                <section class="conflict-overview-group" aria-label={render(group.title)}>
                  <h2>{render(group.title)}</h2>
                  <For each={rows()}>
                    {(conflict) => (
                      <div class="conflict-overview-row">
                        <button
                          class="conflict-overview-open"
                          title={t("conflict-overview-open-the-page-to-resolve-it-shift-click")}
                          onClick={(event) => open(conflict, event)}
                          onAuxClick={(event) => { if (event.button === 1) open(conflict, event); }}
                        >
                          {conflict.page_name}
                        </button>
                        <span class="conflict-overview-source">{conflictSourceLabel(conflict)}</span>
                        <span class="conflict-overview-count">{conflictBlockCount(conflict)}</span>
                        <Show when={conflict.source === "sync-copy" && copyPath(conflict)}>
                          {(path) => (
                            <button class="settings-btn settings-btn-danger" onClick={() => void discardCopy(path(), conflict.page_name)}>{t("conflict-overview-discard-copy")}</button>
                          )}
                        </Show>
                      </div>
                    )}
                  </For>
                  <For each={extra()}>
                    {(copy) => (
                      <div class="conflict-overview-row">
                        <span class="conflict-overview-open conflict-overview-orphan">{copy.base_name}</span>
                        <span class="conflict-overview-source">{t("conflict-overview-orphan-source", { tag: copy.tag || t("conflict-overview-conflict-copy") })}</span>
                        <span class="conflict-overview-count">—</span>
                        <button class="settings-btn settings-btn-danger" onClick={() => void discardCopy(copy.path, copy.base_name)}>{t("conflict-overview-discard-copy")}</button>
                      </div>
                    )}
                  </For>
                </section>
              </Show>
            );
          }}
        </For>
      </Show>
    </div>
  );
}
