// Concord in-page resolver (og family 8c): resolve a conflict AT the page.
//
// The derived queue (`conflictQueue`) says which pages need judgement; this
// renders the one for the page being viewed. Two artifact sources land here: a
// sync tool's conflict copy, and a VCS merge's `<<<<<<<` markers parsed out of
// the file itself. They differ only in where the two sides come from and which
// guarded backend command applies them; the rows are the shared `DiffRowView`.
//
// Nothing here auto-applies. A base (the markers' own `|||||||` ancestor) only
// decides which side arrives PRE-SELECTED. The write happens on the user's click
// through `resolve_sync_conflict` / `resolve_vcs_marker_conflict`: one tine-store
// transaction guarded by the diff's `base_rev`, which stages the replaced bytes
// (the copy, or the marker file) in the recoverable trash in the same commit.
import { Show, For, createEffect, createMemo, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { errorFamily } from "../errorFamily";
import { graphOwner, readOwned, writeOwned } from "../owned";
import { pushToast } from "../toasts";
import { conflictQueue, refreshSyncConflicts, settleArtifactConflict } from "../ui";
import { applyGraphChange, flushPage, isConflicted, isDirty, isSaving } from "../document";
import {
  DiffRowView,
  collectRows,
  countSuggestions,
  humanizeSideLabel,
  seedSuggestedExceptArtifact,
  seedSuggestedOrNoLoss,
  visibleDiffRows,
} from "./DiffRows";
import type { ConflictObject, MergeDecision, SyncConflictDiff } from "../types";

function errorDetail(error: unknown): string {
  // Tauri rejects a `Result<T, String>` with the bare string; keep its text.
  if (error instanceof Error) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  return "unexpected error";
}

/** The side labels the artifact itself supplied, shortened for a row segment. */
function segLabel(text: string, fallback: string): string {
  const trimmed = text.trim();
  if (!trimmed) return fallback;
  return trimmed.length > 18 ? `${trimmed.slice(0, 17)}…` : trimmed;
}

/** What the two sides of this conflict are called, from the queue object. */
export function sideLabels(conflict: ConflictObject): { mine: string; theirs: string; theirsTitle?: string; base?: string } {
  const of = (role: "mine" | "theirs" | "base") => conflict.sides.find((s) => s.role === role)?.label ?? "";
  const markers = conflict.source === "vcs-markers";
  const theirs = humanizeSideLabel(of("theirs") || (markers ? "Merged-in side" : "Conflict copy"));
  return {
    mine: of("mine") || (markers ? "Local side" : "This device"),
    theirs: theirs.text,
    theirsTitle: theirs.title,
    base: of("base") || undefined,
  };
}

/** A diff read that never leaves an errored resource behind: reading an
 *  errored Solid resource throws inside rendering, which can blank the page. */
type DiffRead = { diff: SyncConflictDiff | null; error?: string };

async function readDiff(c: ConflictObject, alive: () => boolean): Promise<DiffRead> {
  const owner = graphOwner(alive);
  try {
    if (c.source === "vcs-markers") {
      const parsed = await readOwned(owner, backend().vcsMarkerConflictDiff(c.page_path));
      return { diff: parsed.kind === "current" ? parsed.value?.diff ?? null : null };
    }
    const copy = c.sides.find((s) => s.role === "theirs")?.path;
    if (!copy) return { diff: null };
    const read = await readOwned(owner, backend().syncConflictDiff(c.page_path, copy));
    return { diff: read.kind === "current" ? read.value : null };
  } catch (e) {
    return { diff: null, error: errorDetail(e) };
  }
}

/** The in-page conflict resolver for the page currently being viewed. */
export function PageConflictResolution(props: { conflict: ConflictObject }): JSX.Element {
  const conflict = () => props.conflict;
  // Kept as plain values: cleanup runs while the surrounding <Show> is being
  // disposed, when reading `props.conflict` again is a stale reactive access.
  const cleanupConflictId = props.conflict.id;
  const cleanupPageName = props.conflict.page_name;
  let mounted = true;
  const labels = createMemo(() => sideLabels(conflict()));
  const [decisions, setDecisions] = createSignal<Record<string, MergeDecision>>({});
  // Page-header properties are one decision for the whole page, not a row.
  const [preChoice, setPreChoice] = createSignal<"mine" | "theirs" | "union">("union");
  const [showUnchanged, setShowUnchanged] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [cursor, setCursor] = createSignal(0);
  let root: HTMLDivElement | undefined;

  const [read, { refetch }] = createResource(() => conflict().id, () => readDiff(conflict(), () => mounted));
  const diffValue = (): SyncConflictDiff | null => read()?.diff ?? null;

  // Row decisions belong to ONE exact pair of texts: every fresh alignment
  // restarts from the suggested resolution (no-loss where there is none).
  let alignment: string | undefined;
  createEffect(() => {
    const current = diffValue();
    if (!current) return;
    const next = `${current.base_rev}\0${current.conflict_rev}`;
    if (alignment !== next) {
      setDecisions(seedSuggestedOrNoLoss(current.rows));
      setPreChoice("union");
      setCursor(0);
    }
    alignment = next;
  });

  const pending = createMemo(() => collectRows(diffValue()?.rows ?? []));
  const suggestedCount = createMemo(() => countSuggestions(diffValue()?.rows ?? []));
  const rows = createMemo(() => visibleDiffRows(diffValue()?.rows ?? [], showUnchanged()));
  const setDecision = (id: string, d: MergeDecision) => setDecisions((m) => ({ ...m, [id]: d }));
  const setAll = (d: MergeDecision) => {
    const next: Record<string, MergeDecision> = {};
    for (const { id } of pending()) next[id] = d;
    setDecisions(next);
  };
  const applyAllSuggested = () => {
    const current = diffValue();
    if (current) setDecisions((prev) => seedSuggestedExceptArtifact(current.rows, { ...prev }));
  };

  /** Move the highlight to the previous/next row that needs a decision. */
  const step = (delta: number) => {
    const list = pending();
    if (!list.length) return;
    const at = (cursor() + delta + list.length) % list.length;
    setCursor(at);
    const el = root?.querySelector(`[data-row-id="${CSS.escape(list[at].id)}"]`);
    el?.scrollIntoView({ block: "center" });
    el?.classList.add("page-conflict-row-focus");
    window.setTimeout(() => el?.classList.remove("page-conflict-row-focus"), 900);
  };

  const apply = async () => {
    const current = diffValue();
    if (!current || read.loading || busy()) return;
    // Plain snapshots only: resolving retires the queue object, which disposes
    // the <Show> that owns `props.conflict`.
    const c = conflict();
    const { source, id, page_name: pageName, page_path: pagePath, kind } = c;
    const copy = c.sides.find((s) => s.role === "theirs")?.path ?? null;
    const owner = graphOwner();
    setBusy(true);
    try {
      // The open editor must not autosave its pre-merge text over the result.
      // Pending edits are saved first; the guarded write below then refuses
      // (`conflict`) if they changed the file the user reviewed.
      if (isConflicted(pageName)) {
        pushToast("Resolve this page’s save conflict first, then apply this resolution.", "info");
        return;
      }
      if (isDirty(pageName) || isSaving(pageName)) {
        await flushPage(pageName);
        alignment = undefined;
        void refetch();
        pushToast("Your latest edit was saved. Review the updated comparison, then apply it again.", "info");
        return;
      }
      const write = source === "vcs-markers"
        ? backend().resolveVcsMarkerConflict(pagePath, decisions(), current.base_rev, ["replace-page"], preChoice())
        : copy
          ? backend().resolveSyncConflict(pagePath, copy, decisions(), current.base_rev, current.conflict_rev, ["replace-page", "delete-page"], preChoice(), current.merge_base_rev)
          : null;
      if (!write) return;
      const result = await writeOwned(owner, write);
      if (result.kind === "stale") return;
      settleArtifactConflict(id);
      // Own-origin writes raise no watcher event, so the open page reloads here
      // through the ordinary external-change rule: a clean page takes the merged
      // file; one edited meanwhile keeps the edit and is marked conflicted.
      await applyGraphChange({ path: pagePath, name: pageName, kind, created: false, removed: false });
      pushToast(source === "vcs-markers" ? `Resolved the merge in “${pageName}”` : `Merged into “${pageName}”`, "success");
      void refreshSyncConflicts();
    } catch (e) {
      if (errorFamily(e) === "conflict") {
        pushToast("The file changed on disk — re-reading it, please redo your choices.", "error");
        alignment = undefined;
        void refetch();
      } else {
        pushToast(`Couldn’t resolve it: ${errorDetail(e)}`, "error");
      }
    } finally {
      if (mounted) setBusy(false);
    }
  };

  // Leaving the page with work outstanding gets a quiet note, never a dialog.
  onCleanup(() => {
    mounted = false;
    if (conflictQueue().some((q) => q.id === cleanupConflictId)) {
      pushToast(`“${cleanupPageName}” still has unresolved conflicts`, "info");
    }
  });

  const markers = () => conflict().source === "vcs-markers";
  return (
    <div class="page-conflict" ref={root} data-source={conflict().source}>
      <div class="page-conflict-head">
        <span class="page-conflict-title">
          {markers() ? "Unresolved merge from your version-control tool" : "Two versions of this page arrived"}
        </span>
        <span class="page-conflict-nav">
          <Show when={pending().length}>
            <span class="page-conflict-count">{pending().length} conflict{pending().length === 1 ? "" : "s"}</span>
            <button class="settings-btn" title="Previous conflict" onClick={() => step(-1)}>↑</button>
            <button class="settings-btn" title="Next conflict" onClick={() => step(1)}>↓</button>
          </Show>
        </span>
      </div>
      <Show when={markers()}>
        <div class="settings-hint page-conflict-refusal">
          This file still contains merge markers, so Tine refuses to save it: rewriting it would
          re-indent the markers and silently lose one side. Choose below and apply to make it an
          ordinary page again.
        </div>
      </Show>
      <div class="page-conflict-legend">
        <span class="page-conflict-side mine">{labels().mine}</span>
        <span class="page-conflict-side theirs" title={labels().theirsTitle}>{labels().theirs}</span>
        <Show when={labels().base}>
          {(base) => <span class="page-conflict-side base">{base()} (used for the suggestions)</span>}
        </Show>
      </div>
      <Show
        when={diffValue()}
        fallback={
          <div class="page-conflict-empty">
            {read.loading
              ? "Reading both versions…"
              : read()?.error
                ? `Couldn’t read this conflict. (${read()!.error})`
                : "Couldn’t read this conflict."}
          </div>
        }
      >
        {(d) => (
          <Show
            when={!d().blocks_identical || d().pre_differs}
            fallback={
              <div class="page-conflict-empty">
                The two versions are identical — nothing to decide.
                <Show when={!markers()}> The copy is safe to discard from the Conflicts overview.</Show>
              </div>
            }
          >
            <div class="sync-merge-toolbar">
              <span class="settings-hint">
                <Show
                  when={suggestedCount()}
                  fallback={<>Nothing was pre-selected — no common version is known, so both sides are kept.</>}
                >
                  {suggestedCount()} of {pending().length} pre-selected from the last version both sides
                  agreed on — review and confirm.
                </Show>
              </span>
              <span class="sync-merge-toolbar-actions">
                <button
                  class="settings-btn"
                  onClick={applyAllSuggested}
                  title="Re-applies Tine's own suggestions. A merge tool's proposed text keeps your current choice."
                >
                  Apply all suggested
                </button>
                <button class="settings-btn" onClick={() => setAll("both")}>Keep both everywhere</button>
                <button class="settings-btn" onClick={() => setAll("mine")} title={labels().mine}>
                  Keep {segLabel(labels().mine, "mine")}
                </button>
                <button class="settings-btn" onClick={() => setAll("theirs")} title={labels().theirsTitle ?? labels().theirs}>
                  Keep {segLabel(labels().theirs, "theirs")}
                </button>
                <label class="sync-merge-showunchanged">
                  <input type="checkbox" checked={showUnchanged()} onChange={(e) => setShowUnchanged(e.currentTarget.checked)} />
                  show unchanged
                </label>
              </span>
            </div>
            <div class="sync-merge-collabels">
              <span>{labels().mine}</span>
              <span>{labels().theirs}</span>
            </div>
            <div class="page-conflict-rows">
              <For each={rows()}>
                {(item) => (
                  <DiffRowView
                    row={item.row}
                    depth={item.depth}
                    decisions={decisions()}
                    setDecision={setDecision}
                    fallback="both"
                    labels={{ mine: segLabel(labels().mine, "Mine"), theirs: segLabel(labels().theirs, "Theirs") }}
                  />
                )}
              </For>
            </div>
            <Show when={d().pre_differs}>
              <div class="sync-merge-preblock">
                <div class="settings-hint">
                  The page’s own properties differ. Keep{" "}
                  <select
                    class="page-conflict-preblock-choice"
                    value={preChoice()}
                    onChange={(e) => setPreChoice(e.currentTarget.value as "mine" | "theirs" | "union")}
                  >
                    <option value="union">both (merge)</option>
                    <option value="mine">{segLabel(labels().mine, "mine")}</option>
                    <option value="theirs">{segLabel(labels().theirs, "theirs")}</option>
                  </select>
                </div>
              </div>
            </Show>
            <div class="page-conflict-foot">
              <span class="settings-hint">
                {markers()
                  ? "Applying writes the merged page without any markers; the file as it was moves to the recoverable trash."
                  : "The copy moves to the recoverable trash once this is applied."}
              </span>
              <button class="settings-btn settings-btn-primary" disabled={busy() || read.loading} onClick={() => void apply()}>
                {busy() ? "Applying…" : "Apply resolution"}
              </button>
            </div>
          </Show>
        )}
      </Show>
    </div>
  );
}
