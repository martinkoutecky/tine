import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { expect, it } from "vitest";

const script = fs.readFileSync(process.env.P1_BENCH_SOURCE ?? "scripts/bench-og-parity.mjs", "utf8");

it("rename completion verifies every one of the 200 fixture references", () => {
  const start = script.indexOf("function verifyRenameReferences(");
  expect(start, "filename visibility cannot prove API completion with 200 rewritten references").toBeGreaterThanOrEqual(0);
  const end = script.indexOf("\n}", start) + 2;
  let missing = -1;
  let reads = 0;
  const verify = vm.runInNewContext(`(${script.slice(start, end)})`, {
    path,
    fs: { readFileSync(file: string) {
      reads++;
      const i = Number(path.basename(file).slice("Bench Ref ".length, -3));
      return `- linked [[${i === missing ? "Bench Hub" : "Bench Hub Renamed"}]]\n- unlinked Bench Hub mention ${i}\n`;
    } },
  }) as (graph: string) => number;
  expect(verify("fixture")).toBe(200);
  expect(reads).toBe(200);
  missing = 199;
  expect(() => verify("fixture")).toThrow("reference 199");
  expect(script.indexOf("verifyRenameReferences(graph);", script.indexOf("const completed =")))
    .toBeLessThan(script.indexOf("result.metrics.rename200Ms = completed.elapsedMs"));
  expect(script).toContain("result.metrics.renameFilenameVisibleMs = performance.now() - started");
});

it("the IPC probe waits for the rename API response, including its body", async () => {
  const start = script.indexOf("async function startProbe(");
  const end = script.indexOf("async function journey(", start);
  let finish: ((response: unknown) => void) | undefined;
  let finishBody: ((body: string) => void) | undefined;
  const win = { fetch: () => new Promise((resolve) => { finish = resolve; }) } as Record<string, any>;
  const startProbe = vm.runInNewContext(`(${script.slice(start, end).trim()})`, {
    window: win, performance: { now: () => 100 }, requestAnimationFrame: () => {},
    document: { addEventListener: () => {} },
  }) as (browser: { execute: (fn: () => unknown) => unknown }) => Promise<unknown>;
  await startProbe({ execute: (fn) => fn() });
  win.__ogBenchProbe.renameSubmittedAt = 25;
  const pending = win.fetch("ipc://localhost/rename_page", {});
  expect(win.__ogBenchProbe.renameResult).toBeUndefined();
  finish!({ clone: () => ({ text: () => new Promise((resolve) => { finishBody = resolve; }) }),
    headers: { get: () => "ok" } });
  await new Promise((resolve) => setImmediate(resolve));
  expect(win.__ogBenchProbe.renameResult).toBeUndefined();
  finishBody!("{}");
  await pending;
  expect(win.__ogBenchProbe.renameResult).toEqual({ elapsedMs: 75, error: null });
});
