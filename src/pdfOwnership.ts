/**
 * Window-local authority for graph-scoped PDF work.
 *
 * Each webview owns its own module instance.  A graph bind publishes a fresh
 * generation even when the root is unchanged (for example, backup restore), so
 * a callback from an older PdfViewer can never borrow authority from a newer
 * binding merely because the asset filename/root happen to match.
 */
export interface PdfOwnership {
  graphRoot: string;
  generation: number;
}

interface PdfParticipant {
  flush(): Promise<boolean>;
  cancel(): void;
}

let generation = 0;
let current: PdfOwnership | null = null;
const participants = new Map<number, Set<PdfParticipant>>();
const mutations = new Map<number, Set<Promise<boolean>>>();

export class StalePdfOwnershipError extends Error {
  constructor() {
    super("stale PDF graph ownership");
    this.name = "StalePdfOwnershipError";
  }
}

export function activatePdfOwnership(graphRoot: string): PdfOwnership {
  current = Object.freeze({ graphRoot, generation: ++generation });
  return current;
}

export function currentPdfOwnership(): PdfOwnership | null {
  return current;
}

export function isPdfOwnershipCurrent(owner: PdfOwnership): boolean {
  return current?.generation === owner.generation && current.graphRoot === owner.graphRoot;
}

export function pdfOwnershipKey(owner: PdfOwnership): string {
  return `${owner.generation}:${owner.graphRoot}`;
}

/** Register one mounted viewer's pending-state flusher and cancellation hook. */
export function registerPdfParticipant(owner: PdfOwnership, participant: PdfParticipant): () => void {
  if (!isPdfOwnershipCurrent(owner)) {
    participant.cancel();
    return () => {};
  }
  let owned = participants.get(owner.generation);
  if (!owned) {
    owned = new Set();
    participants.set(owner.generation, owned);
  }
  owned.add(participant);
  return () => {
    owned?.delete(participant);
    if (owned?.size === 0) participants.delete(owner.generation);
  };
}

/**
 * Start and track a complete PDF mutation under its captured owner.  The
 * operation is deferred by one microtask so it is enrolled before its first
 * graph-scoped backend call.
 */
export function trackPdfMutation<T>(owner: PdfOwnership, operation: () => Promise<T>): Promise<T> {
  if (!isPdfOwnershipCurrent(owner)) return Promise.reject(new StalePdfOwnershipError());

  let settled: Promise<boolean>;
  const result = Promise.resolve().then(() => {
    if (!isPdfOwnershipCurrent(owner)) throw new StalePdfOwnershipError();
    return operation();
  });
  settled = result.then(
    (value) => value !== false,
    () => false,
  ).finally(() => {
    const owned = mutations.get(owner.generation);
    owned?.delete(settled);
    if (owned?.size === 0) mutations.delete(owner.generation);
  });
  let owned = mutations.get(owner.generation);
  if (!owned) {
    owned = new Set();
    mutations.set(owner.generation, owned);
  }
  owned.add(settled);
  return result;
}

/**
 * Flush every mounted viewer, then await all PDF mutations created by that
 * flush (and any mutation already in flight).  A false result leaves the owner
 * active so graph switch/safe-close can follow their established abort policy.
 */
export async function drainPdfWork(): Promise<boolean> {
  const owner = current;
  if (!owner) return true;

  const flushes = [...(participants.get(owner.generation) ?? [])]
    .map((participant) => Promise.resolve().then(() => participant.flush()).catch(() => false));
  if ((await Promise.all(flushes)).some((ok) => !ok)) return false;

  // A completing operation may synchronously enqueue its required neighbor
  // (area image -> sidecar, for example), so drain to quiescence rather than
  // snapshotting once.
  for (let pass = 0; pass < 8; pass++) {
    if (!isPdfOwnershipCurrent(owner)) return false;
    const pending = [...(mutations.get(owner.generation) ?? [])];
    if (!pending.length) return true;
    if ((await Promise.all(pending)).some((ok) => !ok)) return false;
  }
  return (mutations.get(owner.generation)?.size ?? 0) === 0;
}

/** Invalidate first, then synchronously cancel every callback/task it owned. */
export function retirePdfOwnership(): void {
  const owner = current;
  if (!owner) return;
  current = null;
  const owned = [...(participants.get(owner.generation) ?? [])];
  participants.delete(owner.generation);
  for (const participant of owned) {
    try {
      participant.cancel();
    } catch {
      // Cancellation is best-effort and idempotent; the generation guard is the
      // authoritative write barrier even if a third-party task throws here.
    }
  }
}

/** Test-only state reset; production graph transitions use retire/activate. */
export function resetPdfOwnershipForTest(): void {
  retirePdfOwnership();
  current = null;
  participants.clear();
  mutations.clear();
  generation = 0;
}
