import { afterEach, expect, it, vi } from "vitest";
import { queueReferenceRead } from "./referenceAdmission";

afterEach(() => vi.useRealTimers());
it("limits reads to two, prioritizes visible work and drops obsolete work", async () => {
  vi.useFakeTimers();
  const current = new Set([0, 1, 2, 3]);
  const started: number[] = [];
  const finish = new Map<number, (value: number) => void>();
  const tasks = Array.from({ length: 4 }, (_, id) => queueReferenceRead(() => {
    started.push(id);
    return new Promise<number>(resolve => finish.set(id, resolve));
  }, () => current.has(id), () => id === 3 ? 0 : 1));
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
