// Bench-only events for the storage A/B harness (`tine_store::bench_events`,
// `docs/bench-storage-ab.md` on the og-storage-perf-ab branch).

let unavailable = false;

/** Report `ev` to the backend's `TINE_BENCH_EVENTS` file. A no-op unless the
 *  backend set `__TINE_BENCH_EVENTS__` at startup; a failed report stops
 *  further reports for this run, as the IPC diagnostics do. */
export function benchEvent(ev: "unfreeze"): void {
  if (unavailable || !(globalThis as { __TINE_BENCH_EVENTS__?: boolean }).__TINE_BENCH_EVENTS__) return;
  const t = Date.now();
  void import("@tauri-apps/api/core").then((m) => m.invoke<void>("bench_event", { ev, t })).catch(() => {
    unavailable = true;
  });
}
