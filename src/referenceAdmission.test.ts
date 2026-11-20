import { afterEach, expect, it, vi } from "vitest";
import { ReferencePriority, queueReferenceCommit, queueReferenceRead } from "./referenceAdmission";

afterEach(() => vi.useRealTimers());
it("limits reads to two, prioritizes visible work and drops obsolete work", async () => {
  vi.useFakeTimers();
  const current = new Set([0, 1, 2, 3]);
  const started: number[] = [];
  const finish = new Map<number, (value: number) => void>();
  const tasks = Array.from({ length: 4 }, (_, id) => queueReferenceRead(() => {
    started.push(id);
    return new Promise<number>(resolve => finish.set(id, resolve));
  }, () => current.has(id), () => id === 3 ? ReferencePriority.Visible : ReferencePriority.Background));
  await vi.runAllTimersAsync();
  expect(started).toEqual([3, 0]);
  current.delete(0); current.delete(1);
  finish.get(0)!(0); finish.get(3)!(3);
  await vi.runAllTimersAsync();
  expect(started).toEqual([3, 0, 2]);
  finish.get(2)!(2);
  await vi.runAllTimersAsync();
  expect(await Promise.all(tasks)).toEqual([undefined, undefined, 2, 3]);
});

it("gives visible linked references commit precedence and runs one commit each turn", async () => {
  vi.useFakeTimers();
  const committed: ReferencePriority[] = [];
  const priorities = [ReferencePriority.Background, ReferencePriority.Visible, ReferencePriority.Linked];
  for (const priority of priorities) void queueReferenceCommit(() => committed.push(priority), () => true, () => priority);
  await vi.advanceTimersByTimeAsync(17);
  expect(committed).toEqual([ReferencePriority.Linked]);
  await vi.runAllTimersAsync();
  expect(committed).toEqual(priorities.reverse());
});

it.each([16, 32, 64])("checks ownership at dispatch and read completion for a burst of %i groups", async count => {
  vi.useFakeTimers();
  const current = vi.fn(() => true);
  const work = Array.from({ length: count }, (_, id) => queueReferenceRead(async () => id, current, () => 0)
    .then(value => queueReferenceCommit(() => value, current, () => 0)));
  await vi.runAllTimersAsync();
  expect(await Promise.all(work)).toEqual(Array.from({ length: count }, (_, id) => id));
  expect(current).toHaveBeenCalledTimes(3 * count);
});
