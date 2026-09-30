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
import { graphOwner, readOwned, writeOwned, type Owned } from "../owned";
import { pushToast } from "../toasts";
import { conflictQueue, journalConflicts, refreshJournalConflicts, refreshSyncConflicts, settleArtifactConflict } from "../ui";
import { openFile } from "../router";
import { ConflictFileRow } from "./JournalConflictFileRow";
import { applyGraphChange, conflictReason, flushPage, installLiveResolution, isConflicted, isDirty, isSaving, liveConflictDraft, node, sameLiveDraft } from "../document";
import { dismissEarlierDraft } from "../draftStore";
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
  if (conflict.source === "live-save") return { mine: of("mine") || "Your unsaved edits", theirs: of("theirs") || "The file on disk now" };
  return {
    mine: of("mine") || (markers ? "Local side" : "This device"),
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
      const open = c.live?.page ? null : liveConflictDraft(c.page_name);
      const draft = c.live?.page ?? open?.page;
      if (!c.live || !draft) return { diff: null };
      const read = await readOwned(owner, backend().liveConflictDiff(c.page_path, draft, open ? open.baseRev : c.live.base_rev));
      return read.kind === "current" ? { diff: read.value, draft, generation: open?.generation ?? null } : { diff: null };
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
    if (!sentinel || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver((entries) => {
      const entry = entries[entries.length - 1];
      if (!entry) return;
      const above = !entry.isIntersecting && entry.boundingClientRect.top < 0;
      setDocked(above);
      if (!above) setExpanded(false);
    });
    io.observe(sentinel);
    onCleanup(() => io.disconnect());
  });
  createEffect(() => {
    if (!docked()) return;
    measureDock();
    window.addEventListener("resize", measureDock);
    const scroller = inlineSlot?.closest(".main-content");
    const ro = typeof ResizeObserver !== "undefined" && scroller ? new ResizeObserver(measureDock) : undefined;
    if (ro && scroller) ro.observe(scroller);
    onCleanup(() => {
      window.removeEventListener("resize", measureDock);
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
    const owner = graphOwner(() => mounted);
    const refresh = (message: string) => {
      alignment = undefined;
      void refetch();
      pushToast(message, "info");
    };
    // The page's own pending edits are saved first, then the comparison is
    // re-read against them: a resolution never lands over unseen edits.
    const saveThenReview = async (message: string) => {
      const saved = await writeOwned(owner, flushPage(pageName));
      if (saved.kind === "current" && owner()) refresh(message);
    };
    setBusy(true);
    try {
      if (source === "live-save") {
        if (!live || !reviewed) return;
        if (live.restored) {
          // After a restart the editor holds the disk version and the capsule
          // is the only copy of the draft: never resolve over newer edits.
          // Newer edits to the reopened page are saved first and the kept draft
          // is re-reviewed against them (the guarded write would refuse the
          // stale review anyway); a conflict of their own is settled first.
          if (isConflicted(pageName)) {
            pushToast("This reopened page has its own save conflict. Resolve it first, then resolve the kept draft.", "info");
            return;
          }
          if (isDirty(pageName) || isSaving(pageName)) {
            const ed = editingId();
            if (ed && node(ed)?.page === pageName) {
              pushToast("Finish the current edit, then apply this resolution.", "info");
              return;
            }
            await saveThenReview("Your newer edits to this page were saved. Review the kept draft against them, then apply it again.");
            return;
          }
          const result = await writeOwned(owner, backend().resolveLiveConflict(pagePath, reviewed, live.base_rev,
            current.conflict_rev, current.merge_base_rev, decisions(), preChoice()));
          if (result.kind === "stale" || !owner()) return;
          // The guarded commit is the durable resolution; retire the capsule
          // after it (a crash in between offers an already-resolved draft,
          // never loses one), then show the result through the ordinary rule.
          if (live.record_id) await dismissEarlierDraft(live.record_id);
          if (!owner()) return;
          await applyGraphChange({ path: pagePath, name: pageName, kind, created: false, removed: false }, true);
          if (!owner()) return;
          pushToast(`Resolved your kept draft of “${pageName}”`, "success");
          return;
        }
        const ed = editingId();
        if (ed && node(ed)?.page === pageName) {
          pushToast("Finish the current edit, then apply this resolution.", "info");
          return;
        }
        const now = liveConflictDraft(pageName);
        if (!now || now.generation !== reviewedGeneration || !sameLiveDraft(now.page, reviewed)) {
          refresh("Your draft changed. Review the updated comparison, then apply it again.");
          return;
        }
        const result = await writeOwned(owner, backend().resolveLiveConflict(pagePath, reviewed, now.baseRev,
          current.conflict_rev, current.merge_base_rev, decisions(), preChoice()));
        if (result.kind === "stale" || !owner()) return;
        const installed = await installLiveResolution(pageName, now.generation, reviewed, { ...result.value, id: pagePath });
        if (!owner()) return;
        if (installed === "installed") pushToast(`Resolved the conflict in “${pageName}”`, "success");
        else if (installed === "kept") pushToast(`The resolution was written, and your edits made since the review are kept. Review “${pageName}” again.`, "info");
        return;
      }
      // The open editor must not autosave its pre-merge text over the result.
      // Pending edits are saved first; the guarded write below then refuses
      // (`conflict`) if they changed the file the user reviewed.
      if (isConflicted(pageName)) {
        pushToast("Resolve this page’s save conflict first, then apply this resolution.", "info");
        return;
      }
      if (isDirty(pageName) || isSaving(pageName)) {
        await saveThenReview("Your latest edit was saved. Review the updated comparison, then apply it again.");
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
      pushToast(source === "vcs-markers" ? `Resolved the merge in “${pageName}”`
        : source === "duplicate-journal" ? `Folded the other file into “${pageName}”` : `Merged into “${pageName}”`, "success");
      void refreshSyncConflicts();
      // A day with three files still has one to reconcile after this fold.
      if (source === "duplicate-journal") void refreshJournalConflicts();
    } catch (e) {
      if (owner() && errorFamily(e) === "conflict") {
        pushToast("The file changed on disk — re-reading it, please redo your choices.", "error");
        alignment = undefined;
        void refetch();
      } else {
        pushToast(`Couldn’t resolve it: ${errorDetail(e)}`, "error");
      }
    } finally {
      if (owner()) setBusy(false);
    }
  };

  // Leaving the page with work outstanding gets a quiet note, never a dialog.
  onCleanup(() => {
    mounted = false;
    if (conflictQueue().some((q) => q.id === cleanupConflictId)) {
      pushToast(`“${cleanupPageName}” still has unresolved conflicts`, "info");
    }
  });

  // The day's files for the direct per-file actions, from the inventory the
  // graph already refreshes; the SAME rows Settings renders.
  const dayFiles = () => conflict().source === "duplicate-journal"
    ? journalConflicts().find((day) => day.title === conflict().page_name)?.files ?? []
    : [];
  if (conflict().source === "duplicate-journal") void refreshJournalConflicts();
  const reconcileFile = async (op: () => Promise<Owned<void>>, ok: string) => {
    const owner = graphOwner(() => mounted);
    try {
      const result = await op();
      if (result.kind === "stale" || !owner()) return;
      pushToast(ok, "success");
      void refreshJournalConflicts();
      void refreshSyncConflicts();
    } catch (e) {
      pushToast(`Couldn’t do that: ${errorDetail(e)}`, "error");
    }
  };
  const trashDayFile = async (name: string) => {
    const owner = graphOwner(() => mounted);
    const confirmed = await readOwned(owner, backend().confirm(
      `Move the journal file “${name}” to the trash?\n\n` +
        `It's a duplicate of another file for the same day. It moves to logseq/.tine-trash (recoverable).`
    ));
    if (confirmed.kind === "stale" || !owner() || !confirmed.value) return;
    await reconcileFile(() => writeOwned(graphOwner(() => mounted), backend().trashJournalFile(name, "delete-page")), `Moved ${name} to trash`);
  };

  const markers = () => conflict().source === "vcs-markers";
  const conflictTitle = () => markers() ? "Unresolved merge from your version-control tool"
    : conflict().source === "live-save" ? "Your edits and a newer version on disk"
    : conflict().source === "duplicate-journal" ? "This day has more than one file"
    : "Two versions of this page arrived";
  return (
    <>
    <div class="page-conflict-slot" ref={inlineSlot}>
    <div class="page-conflict" ref={root} data-source={conflict().source}>
      <div class="page-conflict-head">
        <span class="page-conflict-title">{conflictTitle()}</span>
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
      <Show when={conflict().source === "duplicate-journal"}>
        <div class="settings-hint page-conflict-files">
          Choosing below folds the other file into this day and moves it to the recoverable trash.
          Or act on a file directly:
        </div>
        <For each={dayFiles()}>
          {(file) => (
            <ConflictFileRow
              file={file}
              parentLayerId="page-conflict"
              onOpen={() => openFile(file.path, conflict().page_name, "journal")}
              onRename={(name) => void reconcileFile(() => writeOwned(graphOwner(() => mounted), backend().renameFileToPage(file.path, name, "rename-page")), `Renamed ${file.name} → ${name}`)}
              onTrash={() => void trashDayFile(file.name)}
            />
          )}
        </For>
      </Show>
      <Show
        when={diffValue()}
        fallback={
          <div class="page-conflict-empty">
            {readResource.loading
              ? "Reading both versions…"
              : conflict().source === "duplicate-journal" && !read()?.error
                ? "These two files can’t be folded together: one is Markdown and the other Org. Use the file actions above."
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
                <Show when={conflict().source === "sync-copy"}> The copy is safe to discard from the Conflicts overview.</Show>
                <Show when={conflict().source === "duplicate-journal"}> The other file is safe to trash above.</Show>
                <Show when={conflict().source === "live-save"}>
                  {conflict().live?.restored ? " The kept draft can be dismissed from Unsaved changes." : " “Use disk version” above loses nothing."}
                </Show>
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
                  : conflict().source === "live-save"
                    ? "Applying writes the merged page only if the file is still the version shown; a newer change refreshes this comparison."
                  : conflict().source === "duplicate-journal"
                    ? "The other file moves to the recoverable trash once this is applied, leaving the day one file."
                    : "The copy moves to the recoverable trash once this is applied."}
              </span>
              <button class="settings-btn settings-btn-primary" disabled={busy() || readResource.loading} onClick={() => void apply()}>
                {busy() ? "Applying…" : "Apply resolution"}
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
            <span class="page-conflict-dockbar-count">{pending().length} to review</span>
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
