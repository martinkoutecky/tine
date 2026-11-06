/** Shared, cancellable reference work. Two native reads may wait concurrently;
 * shell creation and document admission each yield through a real frame/task
 * boundary. Queues hold callbacks only until the next turn or read completion. */
type Job = { current: () => boolean; priority: () => number; run: () => void; drop: () => void };
import { requestFrame } from "./windowRealm";
const reads: Job[] = [];
const commits: Job[] = [];
let activeReads = 0;
let scheduled = false;

function take(queue: Job[]): Job | undefined {
  for (let i = queue.length - 1; i >= 0; i--) if (!queue[i].current()) queue.splice(i, 1)[0].drop();
  let best = 0;
  for (let i = 1; i < queue.length; i++) if (queue[i].priority() < queue[best].priority()) best = i;
  return queue.splice(best, 1)[0];
}
function wake() {
  if (scheduled) return;
  scheduled = true;
  // RAF gives input/layout a turn; the task runs after that frame's callbacks.
  const turn = () => setTimeout(() => {
    scheduled = false;
    take(commits)?.run();
    while (activeReads < 2) {
      const job = take(reads); if (!job) break;
      job.run();
    }
    if (commits.length || (reads.length && activeReads < 2)) wake();
  }, 0);
  let pending = true;
  const once = () => { if (pending) { pending = false; turn(); } };
  requestFrame(undefined, once);
  // A hidden main window must not starve an active satellite's queue.
  setTimeout(once, 32);
}
export function queueReferenceCommit<T>(run: () => T, current: () => boolean, priority: () => number): Promise<T | undefined> {
  return new Promise((resolve, reject) => {
    commits.push({ current, priority, drop: () => resolve(undefined), run: () => {
      try { resolve(run()); } catch (error) { reject(error); }
    } }); wake();
  });
}
export function queueReferenceRead<T>(read: () => Promise<T>, current: () => boolean, priority: () => number): Promise<T | undefined> {
  return new Promise((resolve, reject) => {
    reads.push({ current, priority, drop: () => resolve(undefined), run: () => {
      activeReads++;
      read().then(value => resolve(current() ? value : undefined), reject).finally(() => { activeReads--; wake(); });
    } }); wake();
  });
}
