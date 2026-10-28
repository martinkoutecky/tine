// The routed page's "changed since you last looked" (ADR 0079): one git read per
// trigger, mapped onto the rendered tree.
//
// Reads happen on (1) entering the page, which includes a reload after an
// external change of its file (a reload is a new page instance), (2) the user
// returning to the app (focus / visibility), and (3) a mark or forget. Never on
// a keystroke and never on Tine's own saves: the page's own edits are never an
// input. A reply is dropped when the graph binding, the page instance or a
// newer request has superseded it; a block edited after the request loses its
// mark by comparing raw text, so nothing is re-read to stay honest.
import { batch, createEffect, createMemo, createSignal, on, onCleanup, untrack, type Accessor } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { backend } from "../backend";
import { graphOwner, readOwned } from "../owned";
import { node, seenPageSnapshot } from "../document";
import { pageIdentityKey } from "../pageIdentity";
import { dbg } from "../debug";
import { onAppReturn } from "../windowRealm";
import { emptyMarks, SEEN_PAGE_ROOT, changeCount, marksFromChanges, type RemovedMark, type SeenMarks } from "./marks";
import { noteSeenAnswer, registerDisplayed, seenAvailable, seenRevision } from "./seenState";

export interface PageSeen {
  /** Does this block carry the margin bar? */
  changed(id: string): boolean;
  /** Removed blocks drawn after the block `id` (its previous sibling). */
  removedAfter(id: string): readonly RemovedMark[];
  /** Removed blocks drawn first inside the parent `id` (null: the page itself). */
  removedFirst(id: string | null): readonly RemovedMark[];
  /** Header count; 0 hides the header. */
  count: Accessor<number>;
  identityKnown: Accessor<boolean>;
  unattributed: Accessor<boolean>;
}

/** Track `page()` while it is non-null. `page` must change identity when the
 *  page is reloaded (a new loaded instance); its fields are never read here. */
export function createPageSeen(page: Accessor<{ name: string } | null>): PageSeen {
  const [marks, setMarks] = createStore<SeenMarks>(emptyMarks());
  const [returned, setReturned] = createSignal(0);
  onCleanup(onAppReturn(() => setReturned((n) => n + 1)));
  let sequence = 0;
  onCleanup(() => { sequence++; });

  const read = (name: string, token: number) => {
    const snapshot = untrack(() => seenPageSnapshot(name));
    if (!snapshot) return;
    const owner = graphOwner(() => token === sequence);
    const key = pageIdentityKey(name);
    // Leaving the page must still mark it after this page instance is gone, so the
    // registry holds the binding alone, not this request's token.
    registerDisplayed(name, { key, path: snapshot.path, owner: graphOwner() });
    void readOwned(owner, backend().getPageChanges(key, snapshot.path, snapshot.text)).then((answer) => {
      if (answer.kind === "stale") return;
      noteSeenAnswer(answer.value.available);
      batch(() => setMarks(reconcile(marksFromChanges(answer.value, snapshot.roots))));
    }, (error) => {
      dbg(`seen: reading changes of ${name} failed: ${String(error)}`);
      if (owner()) batch(() => setMarks(reconcile(emptyMarks())));
    });
  };

  createEffect(on([() => page(), returned, seenRevision], ([current]) => {
    const token = ++sequence;
    // The first read of a binding is the availability check; once the graph
    // said it cannot answer, no page asks again.
    if (!current || untrack(seenAvailable) === false) { setMarks(reconcile(emptyMarks())); return; }
    read(current.name, token);
  }));

  const changed = (id: string): boolean => {
    const raw = marks.blocks[id];
    return raw !== undefined && node(id)?.raw === raw;
  };
  const count = createMemo(() => changeCount(marks, (id) => node(id)?.raw));
  return {
    changed,
    removedAfter: (id) => marks.after[id] ?? [],
    removedFirst: (id) => marks.first[id ?? SEEN_PAGE_ROOT] ?? [],
    count,
    identityKnown: () => marks.identityKnown,
    unattributed: () => marks.unattributed,
  };
}
