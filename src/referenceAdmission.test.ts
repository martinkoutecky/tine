import { afterEach, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { queueReferenceCommit, queueReferenceRead } from "./referenceAdmission";

afterEach(() => vi.useRealTimers());
it("limits reads to two, prioritizes visible work and drops obsolete work", async () => {
  vi.useFakeTimers();
  const current = new Set([0, 1, 2, 3]);
  const started: number[] = [];
  const finish = new Map<number, (value: number) => void>();
  const tasks = Array.from({ length: 4 }, (_, id) => queueReferenceRead(() => {
    started.push(id);
    return new Promise<number>(resolve => finish.set(id, resolve));
  }, () => current.has(id), { urgent: () => id === 3 }));
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

it("runs urgent commits first, one each turn, and moves a job when its facts change", async () => {
  vi.useFakeTimers();
  const committed: string[] = [];
  const [late, setLate] = createSignal(false);
  void queueReferenceCommit(() => committed.push("background"), () => true);
  void queueReferenceCommit(() => committed.push("late"), () => true, { urgent: late });
  void queueReferenceCommit(() => committed.push("urgent"), () => true, { urgent: () => true });
  setLate(true);
  await vi.advanceTimersByTimeAsync(17);
  expect(committed).toEqual(["urgent"]);
  await vi.runAllTimersAsync();
  expect(committed).toEqual(["urgent", "late", "background"]);
});

it.each([16, 32, 64])("checks ownership at dispatch and read completion for a burst of %i groups", async count => {
  vi.useFakeTimers();
  const current = vi.fn(() => true);
  const work = Array.from({ length: count }, (_, id) => queueReferenceRead(async () => id, current)
    .then(value => queueReferenceCommit(() => value, current)));
  await vi.runAllTimersAsync();
  expect(await Promise.all(work)).toEqual(Array.from({ length: count }, (_, id) => id));
  expect(current).toHaveBeenCalledTimes(3 * count);
});
