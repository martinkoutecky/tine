import { For, Show, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { graphOwner, readOwned, writeOwned } from "../owned";
import type { PaneRouter } from "../router";
import { pushToast } from "../toasts";
import type { ConflictObject, ConflictSource, SyncConflict } from "../types";
import { conflictQueue, openPageInSidebar, refreshSyncConflicts, syncConflicts } from "../ui";

// Concord overview (og 8c): every page that needs a decision, in one place,
// rendered from the derived conflict queue and never written to the graph. It
// is the INVENTORY; resolution happens on the page, next to the blocks. Rows
// vanish as the queue re-derives after each Apply.

const GROUPS: { source: ConflictSource; title: string }[] = [
  { source: "sync-copy", title: "Sync conflict copies" },
  { source: "vcs-markers", title: "Version-control merge markers" },
];

export function conflictSourceLabel(conflict: ConflictObject): string {
  const side = (role: "mine" | "theirs") => conflict.sides.find((s) => s.role === role)?.label;
  return conflict.source === "sync-copy"
    ? `sync copy · ${side("theirs") ?? "conflict copy"}`
    : `merge markers · ${side("mine") ?? "local"} vs ${side("theirs") ?? "merged-in"}`;
}

/** Absent is not zero: a count Tine did not compute shows "—", never "0". */
export function conflictBlockCount(conflict: ConflictObject): string {
  const count = conflict.block_conflicts;
  if (count === undefined || count === null) return "—";
  return `${count} block${count === 1 ? "" : "s"}`;
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
    const owner = graphOwner(() => alive);
    const name = path.split("/").pop() ?? path;
    const confirmed = await readOwned(owner, backend().confirm(
      `Discard the conflict copy “${name}”?\n\n` +
        `It moves to logseq/.tine-trash (recoverable). The current “${pageName}” is left as-is.`
    ));
    if (confirmed.kind === "stale" || !confirmed.value) return;
    try {
      const result = await writeOwned(owner, backend().trashSyncConflict(path, "delete-page"));
      if (result.kind === "stale") return;
      pushToast(`Discarded ${name}`, "success");
      await readOwned(owner, refreshSyncConflicts());
    } catch (e) {
      pushToast(`Couldn’t discard it: ${String(e)}`, "error");
    }
  };
  const orphans = (): SyncConflict[] => syncConflicts().filter((c) => !c.base_path);
  const open = (conflict: ConflictObject, event: MouseEvent) => {
    const target = { name: conflict.page_name, pageKind: conflict.kind, path: conflict.page_path };
    // Shift- or middle-click keeps the overview in view beside the page.
    if (event.shiftKey || event.button === 1) openPageInSidebar(target);
    else props.router.openPageTarget(target);
  };
  const total = () => conflictQueue().length + orphans().length;
  return (
    <div class="conflict-overview">
      <h1 class="page-title">Conflicts</h1>
      <Show
        when={total() > 0}
        fallback={<p class="conflict-overview-empty">No conflicts. Pages that need a decision appear here.</p>}
      >
        <p class="conflict-overview-hint">
          Each page is resolved on the page itself, block by block. Shift-click opens it in the
          right sidebar so this list stays in view.
        </p>
        <For each={GROUPS}>
          {(group) => {
            const rows = () => conflictQueue().filter((c) => c.source === group.source);
            const extra = () => (group.source === "sync-copy" ? orphans() : []);
            return (
              <Show when={rows().length || extra().length}>
                <section class="conflict-overview-group" aria-label={group.title}>
                  <h2>{group.title}</h2>
                  <For each={rows()}>
                    {(conflict) => (
                      <div class="conflict-overview-row">
                        <button
                          class="conflict-overview-open"
                          title="Open the page to resolve it (shift-click: right sidebar)"
                          onClick={(event) => open(conflict, event)}
                          onAuxClick={(event) => { if (event.button === 1) open(conflict, event); }}
                        >
                          {conflict.page_name}
                        </button>
                        <span class="conflict-overview-source">{conflictSourceLabel(conflict)}</span>
                        <span class="conflict-overview-count">{conflictBlockCount(conflict)}</span>
                        <Show when={conflict.source === "sync-copy" && copyPath(conflict)}>
                          {(path) => (
                            <button class="settings-btn settings-btn-danger" onClick={() => void discardCopy(path(), conflict.page_name)}>
                              Discard copy
                            </button>
                          )}
                        </Show>
                      </div>
                    )}
                  </For>
                  <For each={extra()}>
                    {(copy) => (
                      <div class="conflict-overview-row">
                        <span class="conflict-overview-open conflict-overview-orphan">{copy.base_name}</span>
                        <span class="conflict-overview-source">
                          sync copy · {copy.tag || "conflict copy"} · its page no longer exists
                        </span>
                        <span class="conflict-overview-count">—</span>
                        <button class="settings-btn settings-btn-danger" onClick={() => void discardCopy(copy.path, copy.base_name)}>
                          Discard copy
                        </button>
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
