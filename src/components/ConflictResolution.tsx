import { msg, t, type Msg } from "../i18n";
// Concord in-page resolver (og family 8c): resolve a conflict AT the page.
//
// The derived queue (`conflictQueue`) says which pages need judgement; this
// renders the one for the page being viewed. Two artifact sources land here: a
// sync tool's conflict copy, and a VCS merge's `<<<<<<<` markers parsed out of
// the file itself. They differ only in where the two sides come from and which
// guarded backend command applies them; the rows are the shared `DiffRowView`.
//
// A third source is an editor draft whose save was refused because its file
// changed on disk (`live-save`, og 8e): its "mine" is the draft itself (the open
// editor's, or a capsule kept across a restart), "theirs" the file as it is
// now, and the Concord ledger's copy of the revision it was edited from makes
// the suggestions 3-way. Its Apply writes through `resolve_live_conflict` at
// the reviewed disk revision and installs the result only over the exact draft
// that was reviewed (`installLiveResolution`).
//
// Nothing here auto-applies. A base (the markers' own `|||||||` ancestor) only
// decides which side arrives PRE-SELECTED. The write happens on the user's click
// through `resolve_sync_conflict` / `resolve_vcs_marker_conflict`: one tine-store
// transaction guarded by the diff's `base_rev`, which stages the replaced bytes
// (the copy, or the marker file) in the recoverable trash in the same commit.
import { Show, For, createEffect, createMemo, createResource, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { backend } from "../backend";
import { errorFamily } from "../errorFamily";
import { bindingOwner, graphOwner, readOwned, writeOwned, type Owned } from "../owned";
import { pushToast } from "../toasts";
import { conflictQueue, journalConflicts, refreshJournalConflicts, refreshSyncConflicts, settleArtifactConflict } from "../ui";
import { openFile } from "../router";
import { ConflictFileRow } from "./JournalConflictFileRow";
import { applyGraphChange, applyLiveResolution, conflictReason, flushPage, isConflicted, isDirty, isSaving, liveConflictDraft, node } from "../document";
import { editingId } from "../editorController";
import { readOr } from "../resourceRead";
import {
  DiffRowView,
  collectRows,
  countSuggestions,
  humanizeSideLabel,
  seedSuggestedExceptArtifact,
  seedSuggestedOrNoLoss,
  visibleDiffRows,
} from "./DiffRows";
import type { ConflictObject, MergeDecision, PageDto, SyncConflictDiff } from "../types";
import { listen, newIntersectionObserver, newResizeObserver } from "../windowRealm";

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
  const theirs = humanizeSideLabel(of("theirs") || (markers ? t("conflicts-prose-merged-side") : t("conflicts-prose-conflict-copy")));
  if (conflict.source === "live-save") return { mine: of("mine") || t("live-conflicts-your-unsaved-edits"), theirs: of("theirs") || t("live-conflicts-the-file-on-disk-now") };
  return {
    mine: of("mine") || (markers ? t("conflicts-prose-local-side") : t("mock-conflicts-this-device")),
    theirs: theirs.text,
    theirsTitle: theirs.title,
    base: of("base") || undefined,
  };
}

/** A diff read that never leaves an errored resource behind: reading an
 *  errored Solid resource throws inside rendering, which can blank the page. */
type DiffRead = { diff: SyncConflictDiff | null; error?: string; draft?: PageDto; generation?: number | null };

async function readDiff(c: ConflictObject, alive: () => boolean): Promise<DiffRead> {
  const owner = graphOwner(alive);
  try {
    if (c.source === "live-save") {
      // The exact draft this review shows: Apply writes it only while the
      // editor still holds it, so later typing is never replaced unseen.
      const open = liveConflictDraft(c.page_name);
      if (!c.live || !open) return { diff: null };
      const read = await readOwned(owner, backend().liveConflictDiff(c.page_path, open.page, open.baseRev));
      return read.kind === "current" ? { diff: read.value, draft: open.page, generation: open.generation } : { diff: null };
    }
    if (c.source === "vcs-markers") {
      const parsed = await readOwned(owner, backend().vcsMarkerConflictDiff(c.page_path));
      return { diff: parsed.kind === "current" ? parsed.value?.diff ?? null : null };
    }
    const copy = c.sides.find((s) => s.role === "theirs")?.path;
    if (!copy) return { diff: null };
    // A duplicate day resolves pairwise, the keeper against its FIRST stray;
    // null means a cross-format pair, and the file rows are the whole surface.
    const read = await readOwned(owner, c.source === "duplicate-journal"
      ? backend().duplicateJournalDiff(c.page_path, copy)
      : backend().syncConflictDiff(c.page_path, copy));
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

  // Dock (master 61ea6600c): the panel scrolls away with the top of the page,
  // which on a phone hid a conflict until the user happened to scroll up. Once
  // the panel is ENTIRELY above the viewport a slim bar pins to this pane's
  // scroller; tapping it moves the SAME panel node (decisions and DOM state
  // survive) into a pinned sheet. Fixed, not sticky: WebKitGTK has no scroll
  // anchoring, so an in-flow height swap would jump the content.
  const [docked, setDocked] = createSignal(false);
  const [expanded, setExpanded] = createSignal(false);
  const [dockRect, setDockRect] = createSignal<{ left: number; top: number; width: number } | null>(null);
  let inlineSlot: HTMLDivElement | undefined;
  let sentinel: HTMLDivElement | undefined;
  let sheetEl: HTMLDivElement | undefined;
  const measureDock = () => {
    const r = inlineSlot?.closest(".main-content")?.getBoundingClientRect();
    setDockRect(r ? { left: r.left, top: r.top, width: r.width } : null);
  };
  onMount(() => {
    // The sentinel sits directly below the panel: a half-visible tall panel,
    // or one still below the fold on a short window, does not dock.
    if (!sentinel) return;
    const io = newIntersectionObserver(sentinel, (entries) => {
      const entry = entries[entries.length - 1];
      if (!entry) return;
      const above = !entry.isIntersecting && entry.boundingClientRect.top < 0;
      setDocked(above);
      if (!above) setExpanded(false);
    });
    if (!io) return;
    io.observe(sentinel);
    onCleanup(() => io.disconnect());
  });
  createEffect(() => {
    if (!docked()) return;
    measureDock();
    const stopResize = listen(inlineSlot ?? sentinel, "resize", measureDock);
    const scroller = inlineSlot?.closest(".main-content");
    const ro = scroller ? newResizeObserver(scroller, measureDock) : null;
    if (ro && scroller) ro.observe(scroller);
    onCleanup(() => {
      stopResize();
      ro?.disconnect();
    });
  });
  // One panel node, moved; the vacated slot keeps its height so nothing jumps.
  createEffect(() => {
    const panel = root;
    if (!panel || !inlineSlot) return;
    if (docked() && expanded() && sheetEl) {
      inlineSlot.style.minHeight = `${panel.offsetHeight}px`;
      sheetEl.appendChild(panel);
    } else if (panel.parentElement !== inlineSlot) {
      inlineSlot.appendChild(panel);
      inlineSlot.style.minHeight = "";
    }
  });

  // A live conflict re-reviews when a newer refused save observed another disk
  // revision; every source re-reviews after a refused Apply.
  const [readResource, { refetch }] = createResource(
    () => `${conflict().id}\0${conflict().source === "live-save" ? conflictReason(conflict().page_name)?.observedRev ?? "" : ""}`,
    () => readDiff(conflict(), () => mounted),
  );
  // readOr: a rejected read degrades to "no comparison" (the panel's own
  // "Couldn't read this conflict." row below) instead of throwing into render.
  const read = () => readOr(readResource, undefined, "conflict comparison");
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
    if (!current || readResource.loading || busy()) return;
    // Plain snapshots only: resolving retires the queue object, which disposes
    // the <Show> that owns `props.conflict`.
    const c = conflict();
    const { source, id, page_name: pageName, page_path: pagePath, kind } = c;
    const copy = c.sides.find((s) => s.role === "theirs")?.path ?? null;
    const live = c.live;
    const reviewed = read()?.draft, reviewedGeneration = read()?.generation ?? null;
    const owner = bindingOwner(() => mounted);
    const refresh = (message: Msg) => {
      alignment = undefined;
      void refetch();
      pushToast(message, "info");
    };
    // The page's own pending edits are saved first, then the comparison is
    // re-read against them: a resolution never lands over unseen edits.
    const saveThenReview = async (message: Msg) => {
      const saved = await writeOwned(owner, flushPage(pageName));
      if (saved.kind === "current" && owner()) refresh(message);
    };
    setBusy(true);
    try {
      if (source === "live-save") {
        if (!live || !reviewed || reviewedGeneration === null) return;
        const ed = editingId();
        if (ed && node(ed)?.page === pageName) {
          pushToast(msg("conflict-resolution-finish-the-current-edit-then-apply-this-resolution"), "info");
          return;
        }
        // The merge is computed natively from the reviewed draft and submitted
        // on the reviewed disk state only if no input arrived since the review.
        const outcome = await writeOwned(owner, applyLiveResolution(pageName, reviewedGeneration, reviewed, pagePath,
          current.conflict_rev, current.merge_base_rev, decisions(), preChoice()));
        if (outcome.kind === "stale" || !owner()) return;
        if (outcome.value === "installed") pushToast(msg("conflict-resolution-resolved-the-conflict-in", { page_name: String(pageName) }), "success");
        else if (outcome.value === "saving")
          pushToast(msg("conflict-resolution-applied-the-resolution-to-but-it-is-not", { page_name: String(pageName) }), "info");
        else refresh(msg("conflict-resolution-draft-changed"));
        return;
      }
      // The open editor must not autosave its pre-merge text over the result.
      // Pending edits are saved first; the guarded write below then refuses
      // (`conflict`) if they changed the file the user reviewed.
      if (isConflicted(pageName)) {
        pushToast(msg("conflict-resolution-resolve-this-page-s-save-conflict-first-then"), "info");
        return;
      }
      if (isDirty(pageName) || isSaving(pageName)) {
        await saveThenReview(msg("conflict-resolution-latest-saved"));
        return;
      }
      // A duplicate day reaches the same guarded two-file fold through its own
      // command, whose day/keeper guard keeps it from merging unrelated pages.
      const write = source === "vcs-markers"
        ? backend().resolveVcsMarkerConflict(pagePath, decisions(), current.base_rev, ["replace-page"], preChoice())
        : source === "duplicate-journal" && copy
          ? backend().resolveDuplicateJournalDay(pagePath, copy, decisions(), current.base_rev, current.conflict_rev, ["replace-page", "delete-page"], preChoice())
          : copy
          ? backend().resolveSyncConflict(pagePath, copy, decisions(), current.base_rev, current.conflict_rev, ["replace-page", "delete-page"], preChoice(), current.merge_base_rev)
          : null;
      if (!write) return;
      const result = await writeOwned(owner, write);
      if (result.kind === "stale" || !owner()) return;
      settleArtifactConflict(id);
      // Own-origin writes raise no watcher event, so the open page reloads here
      // through the ordinary external-change rule: a clean page takes the merged
      // file; one edited meanwhile keeps the edit and is marked conflicted.
      await applyGraphChange({ path: pagePath, name: pageName, kind, created: false, removed: false }, true);
      if (!owner()) return;
      pushToast(msg("conflict-resolution-applied", { source, page: pageName }), "success");
      void refreshSyncConflicts();
      // A day with three files still has one to reconcile after this fold.
      if (source === "duplicate-journal") void refreshJournalConflicts();
    } catch (e) {
      if (owner() && errorFamily(e) === "conflict") {
        pushToast(msg("conflict-resolution-the-file-changed-on-disk-re-reading-it"), "error");
        alignment = undefined;
        void refetch();
      } else {
        pushToast(msg("conflict-resolution-couldn-t-resolve-it", { error: String(errorDetail(e)) }), "error");
      }
    } finally {
      if (owner()) setBusy(false);
    }
  };

  // Leaving the page with work outstanding gets a quiet note, never a dialog.
  onCleanup(() => {
    mounted = false;
    if (conflictQueue().some((q) => q.id === cleanupConflictId)) {
      pushToast(msg("conflict-resolution-still-has-unresolved-conflicts", { page: String(cleanupPageName) }), "info");
    }
  });

  // The day's files for the direct per-file actions, from the inventory the
  // graph already refreshes; the SAME rows Settings renders.
  const dayFiles = () => conflict().source === "duplicate-journal"
    ? journalConflicts().find((day) => day.title === conflict().page_name)?.files ?? []
    : [];
  if (conflict().source === "duplicate-journal") void refreshJournalConflicts();
  const reconcileFile = async (op: () => Promise<Owned<void>>, ok: Msg) => {
    const owner = bindingOwner(() => mounted);
    try {
      const result = await op();
      if (result.kind === "stale" || !owner()) return;
      pushToast(ok, "success");
      void refreshJournalConflicts();
      void refreshSyncConflicts();
    } catch (e) {
      pushToast(msg("conflict-resolution-couldn-t-do-that", { error: String(errorDetail(e)) }), "error");
    }
  };
  const trashDayFile = async (name: string) => {
    const owner = bindingOwner(() => mounted);
    const confirmed = await readOwned(owner, backend().confirm(
      t("conflict-resolution-trash-confirmation", { name })
    ));
    if (confirmed.kind === "stale" || !owner() || !confirmed.value) return;
    await reconcileFile(() => writeOwned(bindingOwner(() => mounted), backend().trashJournalFile(name, "delete-page")), msg("conflict-resolution-file-trashed", { name }));
  };

  const markers = () => conflict().source === "vcs-markers";
  const conflictTitle = () => markers() ? t("conflicts-prose-unresolved-merge-from-your-version-control-tool")
    : conflict().source === "live-save" ? t("conflicts-prose-your-edits-and-a-newer-version-on-disk")
    : conflict().source === "duplicate-journal" ? t("conflicts-prose-this-day-has-more-than-one-file")
    : t("conflicts-prose-two-versions-of-this-page-arrived");
  return (
    <>
    <div class="page-conflict-slot" ref={inlineSlot}>
    <div class="page-conflict" ref={root} data-source={conflict().source}>
      <div class="page-conflict-head">
        <span class="page-conflict-title">{conflictTitle()}</span>
        <span class="page-conflict-nav">
          <Show when={pending().length}>
            <span class="page-conflict-count">{t("conflict-resolution-count", { count: pending().length })}</span>
            <button class="settings-btn" title={t("conflict-resolution-previous-conflict")} onClick={() => step(-1)}>↑</button>
            <button class="settings-btn" title={t("conflict-resolution-next-conflict")} onClick={() => step(1)}>↓</button>
          </Show>
        </span>
      </div>
      <Show when={markers()}>
        <div class="settings-hint page-conflict-refusal">{t("conflict-resolution-this-file-still-contains-merge-markers-so-tine")}</div>
      </Show>
      <div class="page-conflict-legend">
        <span class="page-conflict-side mine">{labels().mine}</span>
        <span class="page-conflict-side theirs" title={labels().theirsTitle}>{labels().theirs}</span>
        <Show when={labels().base}>
          {(base) => <span class="page-conflict-side base">{t("conflict-resolution-base-suggestions", { base: base() })}</span>}
        </Show>
      </div>
      <Show when={conflict().source === "duplicate-journal"}>
        <div class="settings-hint page-conflict-files">{t("conflict-resolution-choosing-below-folds-the-other-file-into-this")}</div>
        <For each={dayFiles()}>
          {(file) => (
            <ConflictFileRow
              file={file}
              parentLayerId="page-conflict"
              onOpen={() => openFile(file.path, conflict().page_name, "journal")}
              onRename={(name) => void reconcileFile(() => writeOwned(bindingOwner(() => mounted), backend().renameFileToPage(file.path, name, "rename-page")), msg("conflict-resolution-file-renamed", { old: file.name, name }))}
              onTrash={() => void trashDayFile(file.name)}
            />
          )}
        </For>
      </Show>
      <Show
        when={diffValue()}
        fallback={
          <div class="page-conflict-empty">
            {readResource.loading ? t("conflict-resolution-reading-both-versions") : conflict().source === "duplicate-journal" && !read()?.error ? t("conflict-resolution-these-two-files-can-t-be-folded-together") : read()?.error ? t("conflict-resolution-couldn-t-read-this-conflict", { error: String(read()!.error) }) : t("conflict-resolution-conflict-unavailable")}
          </div>
        }
      >
        {(d) => (
          <Show
            when={!d().blocks_identical || d().pre_differs}
            fallback={
              <div class="page-conflict-empty">{t("conflict-resolution-the-two-versions-are-identical-nothing-to-decide")}<Show when={conflict().source === "sync-copy"}>{t("conflict-resolution-the-copy-is-safe-to-discard-from-the")}</Show>
                <Show when={conflict().source === "duplicate-journal"}>{t("conflict-resolution-the-other-file-is-safe-to-trash-above")}</Show>
                <Show when={conflict().source === "live-save"}>
                  {t("conflict-resolution-use-disk-version-above-loses-nothing")}
                </Show>
              </div>
            }
          >
            <div class="sync-merge-toolbar">
              <span class="settings-hint">
                <Show
                  when={suggestedCount()}
                  fallback={<>{t("conflict-resolution-nothing-was-pre-selected-no-common-version-is")}</>}
                >
                  {t("conflict-resolution-suggested-summary", { suggested: String(suggestedCount()), total: String(pending().length) })}</Show>
              </span>
              <span class="sync-merge-toolbar-actions">
                <button
                  class="settings-btn"
                  onClick={applyAllSuggested}
                  title={t("conflict-resolution-re-applies-tine-s-own-suggestions-a-merge")}
                >{t("conflict-resolution-apply-all-suggested")}</button>
                <button class="settings-btn" onClick={() => setAll("both")}>{t("conflict-resolution-keep-both-everywhere")}</button>
                <button class="settings-btn" onClick={() => setAll("mine")} title={labels().mine}>{t("conflict-resolution-keep-side", { side: segLabel(labels().mine, t("conflicts-prose-mine")) })}
                </button>
                <button class="settings-btn" onClick={() => setAll("theirs")} title={labels().theirsTitle ?? labels().theirs}>{t("conflict-resolution-keep-side", { side: segLabel(labels().theirs, t("conflicts-prose-theirs")) })}
                </button>
                <label class="sync-merge-showunchanged">
                  <input type="checkbox" checked={showUnchanged()} onChange={(e) => setShowUnchanged(e.currentTarget.checked)} />{t("conflict-resolution-show-unchanged")}</label>
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
                    labels={{ mine: segLabel(labels().mine, t("conflicts-prose-mine-short")), theirs: segLabel(labels().theirs, t("conflicts-prose-theirs-short")) }}
                  />
                )}
              </For>
            </div>
            <Show when={d().pre_differs}>
              <div class="sync-merge-preblock">
                <div class="settings-hint">{t("conflict-resolution-the-page-s-own-properties-differ-keep")}{" "}
                  <select
                    class="page-conflict-preblock-choice"
                    value={preChoice()}
                    onChange={(e) => setPreChoice(e.currentTarget.value as "mine" | "theirs" | "union")}
                  >
                    <option value="union">{t("conflict-resolution-both-merge")}</option>
                    <option value="mine">{segLabel(labels().mine, t("conflicts-prose-mine"))}</option>
                    <option value="theirs">{segLabel(labels().theirs, t("conflicts-prose-theirs"))}</option>
                  </select>
                </div>
              </div>
            </Show>
            <div class="page-conflict-foot">
              <span class="settings-hint">
                {markers() ? t("conflict-resolution-applying-writes-the-merged-page-without-any-markers") : conflict().source === "live-save" ? t("conflict-resolution-applying-writes-the-merged-page-only-if-the") : conflict().source === "duplicate-journal" ? t("conflict-resolution-the-other-file-moves-to-the-recoverable-trash") : t("conflict-resolution-the-copy-moves-to-the-recoverable-trash-once")}
              </span>
              <button class="settings-btn settings-btn-primary" disabled={busy() || readResource.loading} onClick={() => void apply()}>
                {busy() ? t("conflict-resolution-applying") : t("conflict-resolution-apply-resolution")}
              </button>
            </div>
          </Show>
        )}
      </Show>
    </div>
    </div>
    <div class="page-conflict-sentinel" ref={sentinel} aria-hidden="true" />
    <Show when={docked()}>
      <div
        class="page-conflict-dock"
        classList={{ expanded: expanded() }}
        style={dockRect() ? { left: `${dockRect()!.left}px`, top: `${dockRect()!.top}px`, width: `${dockRect()!.width}px` } : undefined}
        onKeyDown={(e) => {
          if (e.key === "Escape" && expanded()) {
            e.stopPropagation();
            setExpanded(false);
          }
        }}
      >
        <button class="page-conflict-dockbar" aria-expanded={expanded()} onClick={() => setExpanded(!expanded())}>
          <span class="page-conflict-dockbar-icon" aria-hidden="true">⚠</span>
          <span class="page-conflict-dockbar-title">{conflictTitle()}</span>
          <Show when={pending().length}>
            <span class="page-conflict-dockbar-count">{t("conflict-resolution-review-count", { count: String(pending().length) })}</span>
          </Show>
          <span class="page-conflict-dockbar-chevron" aria-hidden="true">{expanded() ? "▴" : "▾"}</span>
        </button>
        <Show when={expanded()}>
          <div class="page-conflict-sheet" ref={(el) => (sheetEl = el)} />
        </Show>
      </div>
    </Show>
    </>
  );
}
