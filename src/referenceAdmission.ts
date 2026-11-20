/** Shared source-read and commit budgets. Up to two reads run concurrently and
 * one commit runs per frame/task turn. This module alone decides which waiting
 * job runs next: callers state facts (the element the work is for, whether it
 * is urgent) and a job whose host is in view, or that is urgent, goes first.
 * A job moves between the two FIFOs in O(1) when its facts change; dispatch
 * checks each owner once when dequeued. */
import { createComputed, createRoot, createSignal, type Accessor } from "solid-js";
import { observeViewportWindow } from "./viewportWindow";
import { requestFrame } from "./windowRealm";

/** What the caller knows about the work; never a priority. */
export interface Demand { host?: Element; urgent?: () => boolean }
type Job = { current: () => boolean; run: () => void; drop: () => void; release?: () => void; previous?: Job; next?: Job };
type Fifo = { first?: Job; last?: Job };
type Queue = readonly [Fifo, Fifo];
const reads: Queue = [{}, {}];
const commits: Queue = [{}, {}];
let activeReads = 0;
let scheduled = false;

// One observer per host with waiting work, released when its last job leaves.
const hosts = new WeakMap<Element, { inView: Accessor<boolean>; jobs: number; stop: () => void }>();
function watchHost(host: Element) {
  let watch = hosts.get(host);
  if (!watch) {
    const [inView, setInView] = createSignal(false);
    hosts.set(host, watch = { inView, jobs: 0, stop: observeViewportWindow(host as HTMLElement, entry => setInView(entry.isIntersecting), 0) });
  }
  watch.jobs++;
  const current = watch;
  return { inView: current.inView, release: () => { if (--current.jobs === 0) { current.stop(); hosts.delete(host); } } };
}

function unlink(fifo: Fifo, job: Job) {
  if (job.previous) job.previous.next = job.next;
  else fifo.first = job.next;
  if (job.next) job.next.previous = job.previous;
  else fifo.last = job.previous;
  job.previous = job.next = undefined;
}

function enqueue(queue: Queue, job: Job, demand: Demand) {
  const host = demand.host && watchHost(demand.host);
  let fifo: Fifo | undefined;
  job.release = createRoot(dispose => {
    createComputed(() => {
      const next = queue[demand.urgent?.() || host?.inView() ? 0 : 1];
      if (next === fifo) return;
      if (fifo) unlink(fifo, job);
      job.previous = next.last;
      if (next.last) next.last.next = job;
      else next.first = job;
      next.last = job;
      fifo = next;
    });
    return () => { dispose(); host?.release(); };
  }, null);
  wake();
}

function take(queue: Queue): Job | undefined {
  for (const fifo of queue) {
    while (fifo.first) {
      const job = fifo.first;
      unlink(fifo, job);
      job.release?.();
      job.release = undefined;
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
export function queueReferenceCommit<T>(run: () => T, current: () => boolean, demand: Demand = {}): Promise<T | undefined> {
  return new Promise((resolve, reject) => {
    enqueue(commits, { current, drop: () => resolve(undefined), run: () => {
      try { resolve(run()); } catch (error) { reject(error); }
    } }, demand);
  });
}
export function queueReferenceRead<T>(read: () => Promise<T>, current: () => boolean, demand: Demand = {}): Promise<T | undefined> {
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
    } }, demand);
  });
}
