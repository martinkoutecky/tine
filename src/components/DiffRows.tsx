// The block-diff row renderer (og family 8c).
//
// ONE renderer for the ONE resolution surface: the in-page resolver
// (ConflictResolution.tsx). The Settings merge modal that used to carry its own
// copy is retired; two renderers of the same rows drift apart silently.
//
// Rows are rendered as a FLAT list (`visibleDiffRows`), each carrying its
// depth, rather than as a recursive component tree: a diff at the outline
// depth cap must render without a component per nesting level (I-22).
import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import type { DiffRow, MergeDecision, MergedProposal, RowKind } from "../types";

import { appNow } from "../journal";
/** Why this body is on offer. The two sources carry different guarantees:
 *  Tine composed it from two edits that touch different parts of the body, or
 *  the merge tool that left the markers proposed it. Neither is ever applied
 *  without the user's confirmation. */
export function mergedTitle(source: MergedProposal["source"]): string {
  return source === "artifact"
    ? "Proposed by your merge tool's suggested resolution — still applied only when you confirm."
    : "Both edits combined — offered because they touch different parts of the same body";
}

/** The effective decision for a row, given the surface's own default. */
export function decisionOf(
  decisions: Record<string, MergeDecision>,
  id: string,
  fallback: MergeDecision = "mine"
): MergeDecision {
  return decisions[id] ?? fallback;
}

/** The choice that loses NOTHING for a row of this kind: keep both bodies where
 *  both exist, keep a block only one side has. Used where no 3-way suggestion
 *  is available to lead with. */
export function noLossDecision(kind: RowKind): MergeDecision {
  if (kind === "added") return "mine"; // present only here: keeping it loses nothing
  if (kind === "removed") return "theirs"; // present only there: pull it in
  return "both";
}

/** Depth-first, iteratively (I-22): every row of the tree. */
function eachRow(rows: DiffRow[], visit: (row: DiffRow) => void): void {
  const pending = [...rows].reverse();
  while (pending.length) {
    const row = pending.pop()!;
    visit(row);
    for (let i = row.children.length - 1; i >= 0; i--) pending.push(row.children[i]);
  }
}

/** The resolver's opening position: the SUGGESTED resolution wherever the base
 *  justifies one, and the no-loss choice everywhere else. Only a pre-selection;
 *  a `"merged"` suggestion is a suggestion like any other, never an auto-apply. */
export function seedSuggestedOrNoLoss(
  rows: DiffRow[],
  out: Record<string, MergeDecision> = {}
): Record<string, MergeDecision> {
  eachRow(rows, (r) => {
    if (r.kind !== "unchanged") out[r.id] = r.suggestion ?? noLossDecision(r.kind);
  });
  return out;
}

/** The "Apply all suggested" sweep: like [seedSuggestedOrNoLoss], except a row
 *  whose suggestion is a merge TOOL's own text keeps its current decision. The
 *  batch button vouches only for what Tine computed itself. */
export function seedSuggestedExceptArtifact(
  rows: DiffRow[],
  out: Record<string, MergeDecision>
): Record<string, MergeDecision> {
  eachRow(rows, (r) => {
    const artifactMerge = r.suggestion === "merged" && r.merged?.source === "artifact";
    if (r.kind !== "unchanged" && !artifactMerge) out[r.id] = r.suggestion ?? noLossDecision(r.kind);
  });
  return out;
}

/** Every row that needs a decision (id + kind), flattened, depth-first. */
export function collectRows(rows: DiffRow[]): { id: string; kind: RowKind }[] {
  const out: { id: string; kind: RowKind }[] = [];
  eachRow(rows, (r) => {
    if (r.kind !== "unchanged") out.push({ id: r.id, kind: r.kind });
  });
  return out;
}

/** How many rows carry a base-justified suggestion. */
export function countSuggestions(rows: DiffRow[]): number {
  let n = 0;
  eachRow(rows, (r) => {
    if (r.suggestion) n++;
  });
  return n;
}

/** The shown rows in document order with their depth: a hidden unchanged row
 *  hides its subtree. Iterative, so a diff at the outline cap renders (I-22). */
export function visibleDiffRows(rows: DiffRow[], showUnchanged: boolean): { row: DiffRow; depth: number }[] {
  const out: { row: DiffRow; depth: number }[] = [];
  const pending = rows.map((row) => ({ row, depth: 0 })).reverse();
  while (pending.length) {
    const item = pending.pop()!;
    if (!showUnchanged && item.row.kind === "unchanged") continue;
    out.push(item);
    for (let i = item.row.children.length - 1; i >= 0; i--) pending.push({ row: item.row.children[i], depth: item.depth + 1 });
  }
  return out;
}

export function firstLine(text: string): string {
  const l = text.split("\n").find((s) => s.trim().length) ?? "";
  return l.trim();
}

/** Human wording for a sync tool's conflict-copy tag: "Sync copy · Jul 5"
 *  instead of the raw tag, which stays available as the tooltip. */
export function humanizeSideLabel(label: string, now: Date = appNow()): { text: string; title?: string } {
  const syncthing = label.match(/^sync-conflict-(\d{4})(\d{2})(\d{2})-\d{6}-[A-Za-z0-9]+$/);
  const dropbox = label.match(/conflicted copy (\d{4})-(\d{2})-(\d{2})/i);
  const m = syncthing ?? dropbox;
  if (!m) return { text: label };
  const [, y, mo, d] = m;
  const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
  const month = months[Number(mo) - 1] ?? mo;
  const year = now.getFullYear() === Number(y) ? "" : ` ${y}`;
  return { text: `Sync copy · ${month} ${Number(d)}${year}`, title: label };
}

/** The line index a collapsed row previews: the first line that differs, so a
 *  multi-line body that differs far down is not previewed as two equal lines.
 *  Returns 0 whenever the first non-blank lines already differ. */
export function firstDifferingLine(mine: string, theirs: string): number {
  if (firstLine(mine) !== firstLine(theirs)) return 0;
  const a = mine.split("\n");
  const b = theirs.split("\n");
  const max = Math.max(a.length, b.length);
  for (let i = 0; i < max; i++) {
    if (a[i] !== b[i]) return i;
  }
  return 0;
}

/** What one column shows for a collapsed row: its line `k`, trimmed, or `null`
 *  when this side has no such line. `k === 0` is `firstLine` verbatim. */
export function previewLine(text: string, k: number): string | null {
  if (k === 0) return firstLine(text);
  const lines = text.split("\n");
  return k < lines.length ? lines[k].trim() : null;
}

/** Whether the collapsed preview can be the whole story. */
export function needsExpander(texts: (string | null | undefined)[], previewIndex: number): boolean {
  const bodies = texts.filter((t): t is string => t != null).map((t) => t.split("\n"));
  if (bodies.length < 2) return false;
  if (bodies.some((b) => b.length > 1)) return true;
  const max = bodies.reduce((n, b) => Math.max(n, b.length), 0);
  for (let i = 0; i < max; i++) {
    if (i === previewIndex) continue;
    if (bodies.some((b) => b[i] !== bodies[0][i])) return true;
  }
  return false;
}

/** Per body, which of its lines are NOT identical across every body at the same index. */
export function differingLineFlags(bodies: string[][]): boolean[][] {
  const max = bodies.reduce((n, b) => Math.max(n, b.length), 0);
  const shared: boolean[] = [];
  for (let i = 0; i < max; i++) shared.push(bodies.every((b) => b[i] === bodies[0][i]));
  return bodies.map((b) => b.map((_, i) => !shared[i]));
}

/** Per-row segment wording: the names the artifact itself gave its sides. */
export interface DiffRowLabels {
  mine: string;
  theirs: string;
}

/** One diff row. Its children are separate entries of `visibleDiffRows`. */
export function DiffRowView(props: {
  row: DiffRow;
  depth: number;
  decisions: Record<string, MergeDecision>;
  setDecision: (id: string, d: MergeDecision) => void;
  /** Decision assumed for a row the user hasn't touched. */
  fallback?: MergeDecision;
  labels: DiffRowLabels;
}): JSX.Element {
  const row = () => props.row;
  const dec = () => decisionOf(props.decisions, row().id, props.fallback ?? "mine");
  // Expansion is per-row and opt-in, so a logbook-heavy block cannot claim a
  // phone screen until the user asks it to.
  const [expanded, setExpanded] = createSignal(false);
  const seg = (value: MergeDecision, label: string, side: "mine" | "theirs" | "merged", short?: string) => (
    <button
      class="sync-merge-seg"
      classList={{ active: dec() === value }}
      data-side={side}
      data-decision={value}
      title={short && short !== label ? label : undefined}
      onClick={() => props.setDecision(row().id, value)}
    >
      <Show when={short} fallback={label}>
        <span class="sync-merge-seg-dot" data-side={side} aria-hidden="true" />
        <span class="sync-merge-seg-long">{label}</span>
        <span class="sync-merge-seg-short">{short}</span>
      </Show>
    </button>
  );
  // Both columns and the merged strip preview the SAME line index.
  const previewIndex = createMemo(() => {
    const r = row();
    return r.mine && r.theirs ? firstDifferingLine(r.mine.text, r.theirs.text) : 0;
  });
  const preview = (text: string) => {
    const k = previewIndex();
    const line = previewLine(text, k);
    return (
      <>
        <Show when={k > 0}>
          <span class="sync-merge-elided" title="Earlier lines are the same on both sides">…</span>
        </Show>
        {line === null ? <span class="sync-merge-absent">—</span> : line}
      </>
    );
  };
  const bodies = createMemo(() => {
    const r = row();
    const out: { side: "mine" | "theirs" | "merged"; label: string; lines: string[] }[] = [];
    if (r.mine) out.push({ side: "mine", label: props.labels.mine, lines: r.mine.text.split("\n") });
    if (r.theirs) out.push({ side: "theirs", label: props.labels.theirs, lines: r.theirs.text.split("\n") });
    if (r.merged) out.push({ side: "merged", label: "Merged", lines: r.merged.text.split("\n") });
    return out;
  });
  const expandable = createMemo(() =>
    row().kind === "modified"
    && needsExpander([row().mine?.text, row().theirs?.text, row().merged?.text], previewIndex()));
  const lineCount = createMemo(() => bodies().reduce((n, b) => Math.max(n, b.lines.length), 0));
  const expandedBodies = createMemo(() => {
    if (!expanded()) return [];
    const bs = bodies();
    const flags = differingLineFlags(bs.map((b) => b.lines));
    return bs.map((b, i) => ({ ...b, flags: flags[i] }));
  });
  return (
    <div class="sync-merge-row" data-kind={row().kind} data-row-id={row().id} style={{ "padding-left": `${props.depth * 16}px` }}>
      <div class="sync-merge-cols">
        <div class="sync-merge-cell mine" classList={{ chosen: row().kind !== "removed" && dec() !== "theirs" }}>
          {row().mine ? preview(row().mine!.text) : <span class="sync-merge-absent">—</span>}
          <Show when={(row().mine?.child_count ?? 0) > 0}>
            <span class="sync-merge-kids"> +{row().mine!.child_count}</span>
          </Show>
        </div>
        <div class="sync-merge-cell theirs" classList={{ chosen: dec() === "theirs" || dec() === "both" }}>
          {row().theirs ? preview(row().theirs!.text) : <span class="sync-merge-absent">—</span>}
          <Show when={(row().theirs?.child_count ?? 0) > 0}>
            <span class="sync-merge-kids"> +{row().theirs!.child_count}</span>
          </Show>
        </div>
      </div>
      <div class="sync-merge-controls">
        <Show when={row().kind === "modified"}>
          {seg("mine", props.labels.mine, "mine", "Mine")}
          {seg("theirs", props.labels.theirs, "theirs", "Theirs")}
          {seg("both", "Both", "theirs")}
          <Show when={row().merged}>{seg("merged", "Merged", "merged")}</Show>
        </Show>
        <Show when={row().kind === "added"}>
          {seg("mine", "Keep", "mine")}
          {seg("theirs", "Drop", "theirs")}
        </Show>
        <Show when={row().kind === "removed"}>
          {seg("mine", "Skip", "mine")}
          {seg("theirs", "Pull in", "theirs")}
        </Show>
        <Show when={row().kind === "unchanged"}>
          <span class="sync-merge-unchanged-tag">unchanged</span>
        </Show>
        <Show when={row().suggestion && dec() === row().suggestion}>
          <span class="sync-merge-suggested-tag" title="Pre-selected from the last version both sides agreed on">
            suggested
          </span>
        </Show>
        <Show when={expandable()}>
          <button
            class="sync-merge-expand"
            title={expanded() ? "Hide the full bodies" : `Show all ${lineCount()} lines`}
            aria-expanded={expanded()}
            onClick={() => setExpanded(!expanded())}
          >
            {expanded() ? "⌃" : `⌄ ${lineCount()}`}
          </button>
        </Show>
      </div>
      <Show when={row().kind === "modified" ? row().merged : null}>
        {(merged) => (
          <div
            class="sync-merge-cell merged"
            classList={{ chosen: dec() === "merged" }}
            data-side="merged"
            data-source={merged().source}
            title={mergedTitle(merged().source)}
          >
            <span class="sync-merge-mergedtag">{merged().source === "artifact" ? "Merged (tool)" : "Merged"}</span>
            {preview(merged().text)}
          </div>
        )}
      </Show>
      <Show when={expanded()}>
        <div class="sync-merge-expanded">
          <For each={expandedBodies()}>
            {(body) => (
              <div class="sync-merge-fulltext" data-side={body.side}>
                <div class="sync-merge-fulltext-label">{body.label}</div>
                <div class="sync-merge-fulltext-body">
                  <For each={body.lines}>
                    {(line, i) => (
                      <div class="sync-merge-fulltext-line" classList={{ differs: body.flags[i()] }}>{line}</div>
                    )}
                  </For>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
