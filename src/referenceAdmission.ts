/** Shared source-read and commit budgets. Up to two reads run concurrently and
 * one commit runs per frame/task turn. Waiting jobs move to the tail of their
 * new priority in O(1); dispatch checks each owner once when dequeued. */
import { createComputed, createRoot } from "solid-js";
import { requestFrame } from "./windowRealm";

export const ReferencePriority = { Linked: 0, Visible: 1, Background: 2 } as const;
export type ReferencePriority = typeof ReferencePriority[keyof typeof ReferencePriority];

type Job = { current: () => boolean; run: () => void; drop: () => void; stopPriority?: () => void; previous?: Job; next?: Job };
type Fifo = { first?: Job; last?: Job };
type Queue = readonly [Fifo, Fifo, Fifo];
const reads: Queue = [{}, {}, {}];
const commits: Queue = [{}, {}, {}];
let activeReads = 0;
let scheduled = false;

function unlink(fifo: Fifo, job: Job) {
  if (job.previous) job.previous.next = job.next;
  else fifo.first = job.next;
  if (job.next) job.next.previous = job.previous;
  else fifo.last = job.previous;
  job.previous = job.next = undefined;
}

function enqueue(queue: Queue, job: Job, priority: () => ReferencePriority) {
  let fifo: Fifo | undefined;
  job.stopPriority = createRoot(dispose => {
    createComputed(() => {
      const next = queue[priority()];
      if (next === fifo) return;
      if (fifo) unlink(fifo, job);
      job.previous = next.last;
      if (next.last) next.last.next = job;
      else next.first = job;
      next.last = job;
      fifo = next;
    });
    return dispose;
  }, null);
  wake();
}

function take(queue: Queue): Job | undefined {
  for (const fifo of queue) {
    while (fifo.first) {
      const job = fifo.first;
      unlink(fifo, job);
      job.stopPriority?.();
      job.stopPriority = undefined;
      if (job.current()) return job;
      job.drop();
    }
  }
}

function wake() {
  if (scheduled || (!commits.some(fifo => fifo.first) && (activeReads >= 2 || !reads.some(fifo => fifo.first)))) return;
  scheduled = true;
  // RAF gives input/layout a turn; the task runs after that frame's callbacks.
  const turn = () => setTimeout(() => {
    scheduled = false;
    take(commits)?.run();
    while (activeReads < 2) {
      const job = take(reads); if (!job) break;
      job.run();
    }
    wake();
  }, 0);
  let pending = true;
  const once = () => { if (pending) { pending = false; turn(); } };
  requestFrame(undefined, once);
  // A hidden main window must not starve an active satellite's queue.
  setTimeout(once, 32);
}
export function queueReferenceCommit<T>(run: () => T, current: () => boolean, priority: () => ReferencePriority): Promise<T | undefined> {
  return new Promise((resolve, reject) => {
    enqueue(commits, { current, drop: () => resolve(undefined), run: () => {
      try { resolve(run()); } catch (error) { reject(error); }
    } }, priority);
  });
}
export function queueReferenceRead<T>(read: () => Promise<T>, current: () => boolean, priority: () => ReferencePriority): Promise<T | undefined> {
  return new Promise((resolve, reject) => {
    enqueue(reads, { current, drop: () => resolve(undefined), run: async () => {
      activeReads++;
      try {
        const value = await read();
        resolve(current() ? value : undefined);
      } catch (error) {
        reject(error);
      } finally {
        activeReads--;
        wake();
      }
    } }, priority);
  });
}
