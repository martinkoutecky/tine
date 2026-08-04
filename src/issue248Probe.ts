// GH #248's manual native benchmark installs this collector before exercising a
// routed editor. The production path is a no-op unless that benchmark explicitly
// supplies the collector, so it observes existing ordering and durability rather
// than introducing a new save path or scheduling policy.

export interface Issue248BenchCollector {
  record(metric: string, valueMs: number): void;
}

declare global {
  interface Window {
    __tineIssue248Bench?: Issue248BenchCollector;
  }
}

export function issue248Collector(): Issue248BenchCollector | undefined {
  return typeof window === "undefined" ? undefined : window.__tineIssue248Bench;
}

export function issue248Now(): number {
  return typeof performance === "undefined" ? Date.now() : performance.now();
}

export function measureIssue248<T>(metric: string, work: () => T): T {
  const collector = issue248Collector();
  if (!collector) return work();
  const started = issue248Now();
  try {
    return work();
  } finally {
    collector.record(metric, issue248Now() - started);
  }
}

export async function measureIssue248Async<T>(metric: string, work: () => Promise<T>): Promise<T> {
  const collector = issue248Collector();
  if (!collector) return work();
  const started = issue248Now();
  try {
    return await work();
  } finally {
    collector.record(metric, issue248Now() - started);
  }
}
