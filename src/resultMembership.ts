import { createMemo, createResource, onCleanup, untrack, type Accessor, type Resource } from "solid-js";
import { editingId } from "./editorController";
import { graphEpoch } from "./graphSession";
import { changed } from "./changeKeys";
import { node as docNode, isConflicted, isDirty, isSaving, pendingDataRevision, pageInstanceGeneration } from "./document";
import { graphOwner, readOwned } from "./owned";
import { readOr, readLatestOr } from "./resourceRead";
import { bindingIdentity } from "./binding";
import type { BlockDto, RefGroup } from "./types";
import { adoptResults, captureResultsTicket, resultsEpoch, resultsSettled } from "./queryResultsEpoch";
import { resultDrafts } from "./queryResultDrafts";

export function blocksContainEdit(blocks: BlockDto[], id: string): boolean {
  return blocks.some((block) => block.id === id || blocksContainEdit(block.children, id));
}

/** Whether editing `id` is editing these results. A run's rows are shallow DTOs, and the rows an expanded
 * result shows beneath a hit are its live source blocks, so a block below a listed row counts too (GH #851):
 * walking up from it, the first listed ancestor makes it part of the answer being held. */
export function groupsContainEdit(groups: RefGroup[], id: string): boolean {
  if (groups.some((group) => blocksContainEdit(group.blocks, id))) return true;
  const seen = new Set<string>([id]);
  for (let parent = docNode(id)?.parent; parent && !seen.has(parent); parent = docNode(parent)?.parent) {
    seen.add(parent);
    if (groups.some((group) => blocksContainEdit(group.blocks, parent))) return true;
  }
  return false;
}

/** One read lifecycle for derived result surfaces. Ordinary autosave is untouched.
 * Intentional difference: OG re-runs custom queries mid-edit (react.cljs:335).
 * Tine keeps the rendered answer/position until editing ends and the final save
 * revision settles, then coalesces suppressed invalidations into one request.
 * Identity changes always retire retention; an in-flight read cannot replace an
 * answer underneath its editor or land in a different graph/route (I-20).
 * A deferred load (`undefined`) or rejected adoption keeps the same scope's
 * previous answer; an accepted empty collection replaces it. No extra read. */
export function createMembershipResource<S, T>(
  source: Accessor<S | undefined | null | false>,
  identity: Accessor<string>,
  load: (source: S) => Promise<T>,
  contains: (value: T, id: string) => boolean,
  invalidate: Accessor<boolean> = () => true,
  guardResults = false,
  // An unclassified caller must conservatively refresh on every publication.
  revisionSource: Accessor<unknown> = () => changed("publication"),
  // A precisely classified reader's publication key owns invalidation. Global
  // hydration epochs still gate adoption/actions, but unrelated page reloads
  // need not execute its question again. `current` excludes coalesced changes.
  scopedRevision?: { current: Accessor<boolean> },
) {
  let snapshot: T | undefined;
  let scope = "";
  let retained: { id: string; page?: string; instance: number | null } | undefined;
  let snapshotEpoch = -1;
  let release = 0;
  let alive = true;
  let resolvedScope = "";
  let answered: { input: S | undefined | null | false; scope: string; revision: unknown; epoch: number; release: number; guarded: boolean } | undefined;
  onCleanup(() => { alive = false; });
  const inputs = createMemo(source);
  const currentScope = () => `${bindingIdentity()}\0${graphEpoch()}\0${identity()}`;
  const request = createMemo(() => {
    const nextScope = currentScope();
    const input = inputs();
    const revision = invalidate() ? revisionSource() : 0;
    // A suppressed (collapsed) read is display-only: never stamped, so never acted on.
    const guarded = guardResults && invalidate();
    const epoch = guarded ? resultsEpoch() : 0;
    const settled = !guarded || resultsSettled();
    const id = editingId();
    if (scope !== nextScope) {
      scope = nextScope;
      snapshot = undefined;
      retained = undefined;
    }
    const draft = snapshot === undefined ? undefined : resultDrafts().find(draft => untrack(() => contains(snapshot!, draft.id)));
    if (draft) return { input, scope, revision, epoch, settled, guarded, release, held: true };
    // Local structure changes move the answer epoch without replacing its
    // source. Keep the editing occurrence through an Enter handoff to a live
    // descendant; replacement still retires it, as live-row admission does.
    // Without a loaded source, only the answer's original epoch proves identity.
    if (retained && guarded && (retained.page
      ? retained.instance !== pageInstanceGeneration(retained.page) || docNode(retained.id)?.page !== retained.page
      : snapshotEpoch !== epoch)) {
      retained = undefined; snapshotEpoch = -1; release++;
    }
    if (id && snapshot !== undefined && (!guarded || snapshotEpoch === epoch || retained) && untrack(() => contains(snapshot!, id))) {
      const page = untrack(() => docNode(id)?.page);
      retained = { id, page, instance: guarded && page ? pageInstanceGeneration(page) : null };
    }
    if (retained) {
      const page = retained.page;
      const unsettled = page && (isDirty(page) || isSaving(page) || isConflicted(page) || pendingDataRevision());
      if (id === retained.id || unsettled) return { input, scope, revision, epoch, settled, guarded, release, held: true };
      retained = undefined;
      release++;
    }
    return { input, scope, revision, epoch, settled, guarded, release, held: false };
  });
  // Holding never starts another scan. Keep a stable source token during the
  // session, even across many autosave revisions; release starts exactly once.
  const runnable = createMemo((previous: ReturnType<typeof request> | undefined) => {
    const next = request();
    if (next.held && previous?.scope === next.scope) return previous;
    return next;
  }, undefined, { equals: (a, b) => a?.scope === b?.scope && a?.input === b?.input
    && a?.revision === b?.revision && a?.release === b?.release && (scopedRevision || a?.epoch === b?.epoch) && a?.settled === b?.settled && a?.guarded === b?.guarded });
  const [resource, actions] = createResource(
    () => runnable()?.input && runnable()?.settled ? runnable() : undefined,
    async (started) => {
      // A permission gate (e.g. clean host Open) does not change the question or
      // its answer. Reuse this resource's current snapshot, without an IPC read.
      if (answered && snapshot !== undefined && answered.scope === started.scope
        && answered.input === started.input && answered.revision === started.revision
        && (scopedRevision || answered.epoch === started.epoch) && answered.release === started.release
        && answered.guarded === started.guarded) {
        // Only renew provenance when the native dependency key proves that
        // this answer is unchanged. A delayed relevant refresh keeps its old
        // ticket and the existing live-source check continues to refuse edits.
        if (started.guarded && scopedRevision?.current()) {
          const ticket = captureResultsTicket();
          adoptResults(snapshot, ticket);
          snapshotEpoch = ticket.epoch;
        }
        return snapshot;
      }
      const owner = graphOwner(() => alive && request().scope === started.scope
        && !request().held && runnable() === started);
      if (request().held) return snapshot;
      const ticket = captureResultsTicket();
      const result = await readOwned(owner, load(started.input as S));
      if (result.kind === "stale") return request().scope === started.scope && request().held ? snapshot : undefined;
      if (request().held) return snapshot;
      if (runnable() !== started) return undefined;
      if (result.value === undefined) return snapshot;
      if (started.guarded && adoptResults(result.value, ticket) === undefined) return snapshot;
      snapshot = result.value;
      answered = started;
      snapshotEpoch = ticket.epoch;
      resolvedScope = started.scope;
      return result.value;
    },
  );
  const visible = (() => {
    const value = readOr(resource, undefined, "result membership");
    return resolvedScope === currentScope() ? value : undefined;
  }) as Resource<T | undefined>;
  Object.defineProperties(visible, {
    latest: { get: () => {
      const value = readLatestOr(resource, undefined, "result membership");
      return resolvedScope === currentScope() ? value : undefined;
    } },
    loading: { get: () => resource.loading },
    error: { get: () => resource.error },
    state: { get: () => resource.state },
  });
  return [visible, actions] as const;
}
