// Main-thread driver for the mldoc comparison worker. Owns worker lifecycle and
// the per-parse timeout (a synchronous mldoc hang never returns a message, so the
// timeout fires on THIS side and we terminate the worker — the browser analog of
// graph-check.mjs killing a wedged subprocess).
//
// Two entry points mirror graph-check's warm-vs-fresh distinction:
//   parseWarm  — reuse one long-lived worker for the fast first scan (a prior
//                parse may contaminate the next; that's why the scan is only a
//                candidate filter, re-verified fresh before anything is reported).
//   parseFresh — a brand-new worker (uncontaminated mldoc realm) for the
//                authoritative re-verify / minimize / anonymize probes.
import mldocUrl from "./vendor/mldoc.js?url";
import type { ParseRequest, ParseResponse } from "./worker";

export type Format = "md" | "org";
export interface Projection {
  blocks: unknown[];
  refs: { page: string[]; block: string[] };
}
export type ParseResult =
  | { ok: true; projection: Projection; parseMicros: number }
  | { ok: false; status: string; detail: string };

function newWorker(): Worker {
  return new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
}

function parseOn(worker: Worker, text: string, format: Format, timeoutMs: number): Promise<ParseResult> {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (r: ParseResult) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      worker.onmessage = null;
      worker.onerror = null;
      resolve(r);
    };
    const timer = setTimeout(() => finish({ ok: false, status: "timeout", detail: `timeout after ${timeoutMs}ms` }), timeoutMs);
    worker.onmessage = (ev: MessageEvent<ParseResponse>) => {
      const r = ev.data;
      if (r.ok && r.projection) finish({ ok: true, projection: r.projection, parseMicros: r.parseMicros ?? 0 });
      else finish({ ok: false, status: r.status ?? "error", detail: r.detail ?? "mldoc failed" });
    };
    worker.onerror = (e) => finish({ ok: false, status: "worker-error", detail: String((e as ErrorEvent).message || e) });
    const msg: ParseRequest = { type: "parse", id: "p", mldocUrl, text, format };
    worker.postMessage(msg);
  });
}

export class MldocClient {
  private warm: Worker | null = null;

  /** Fast scan parse — reuses one worker. On timeout/crash the worker is likely
   *  wedged (sync parse), so recycle it before the next call. */
  async parseWarm(text: string, format: Format, timeoutMs: number): Promise<ParseResult> {
    if (!this.warm) this.warm = newWorker();
    const r = await parseOn(this.warm, text, format, timeoutMs);
    if (!r.ok && (r.status === "timeout" || r.status === "worker-error")) this.recycleWarm();
    return r;
  }

  /** Authoritative parse in a fresh mldoc realm (no cross-parse contamination). */
  async parseFresh(text: string, format: Format, timeoutMs: number): Promise<ParseResult> {
    const w = newWorker();
    try {
      return await parseOn(w, text, format, timeoutMs);
    } finally {
      w.terminate();
    }
  }

  private recycleWarm() {
    if (this.warm) {
      this.warm.terminate();
      this.warm = null;
    }
  }

  dispose() {
    this.recycleWarm();
  }
}
