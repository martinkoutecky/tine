// @vitest-environment jsdom
// Shown text step 2a (REG-FIND-REPLACE-WRITES-ONLY-SHOWN-TEXT), cost: planning
// one block renders its text once to find the matches and once for the joint
// proof, and maps every span end to source bytes in one kernel call, however
// many matches the block holds and however many of them are not drawn. Fails on
// the first version of the gate, which made one proof render per hit when one
// match was refused (N + 2 renders).
import { beforeAll, describe, expect, it, vi } from "vitest";

const calls = vi.hoisted(() => ({ mapping: 0 }));
vi.mock("./render/parse", async (original) => {
  const real = await original<typeof import("./render/parse")>();
  return { ...real, rawBytesOfEditor: (...args: Parameters<typeof real.rawBytesOfEditor>) => { calls.mapping++; return real.rawBytesOfEditor(...args); } };
});

import { initParser } from "./render/parse";
import { renderedBlockTextCallCountForTests } from "./render/renderedText";
import { planBlockReplacement } from "./inpageReplace";
import { utf8ByteLength } from "./render/spans";

beforeAll(async () => { await initParser(); });

/** What the pane draws as itself in this block: every source byte before the
 *  embed macro (source bytes count from the parser's "- " bullet). */
const drawnBefore = (prefix: string) => ([, to]: [number, number]) => to <= utf8ByteLength(prefix) + 2;

describe.each(["md", "org"] as const)("%s: Replace all in one block costs two renders and one mapping", (format) => {
  for (const n of [400, 800, 1600]) {
    it(`${n} drawn needles and one embed target: 2 renders, 1 mapping`, () => {
      const prefix = "needle ".repeat(n);
      const raw = `${prefix}{{embed [[needle]]}}`;
      const renders = renderedBlockTextCallCountForTests();
      calls.mapping = 0;
      const t0 = performance.now();
      const plan = planBlockReplacement(raw, format, "needle", "X", "all", true, drawnBefore(prefix));
      const ms = performance.now() - t0;
      expect(plan).toEqual({ raw: `${"X ".repeat(n)}{{embed [[needle]]}}`, replaced: n, skipped: 1 });
      expect(renderedBlockTextCallCountForTests() - renders).toBe(2);
      expect(calls.mapping).toBe(1);
      console.info(`[cost] ${format} N=${n} one refused: ${ms.toFixed(1)} ms`);
    });

    it(`${n} drawn needles, all accepted: 2 renders, 1 mapping`, () => {
      const raw = "needle ".repeat(n).trimEnd();
      const renders = renderedBlockTextCallCountForTests();
      calls.mapping = 0;
      const plan = planBlockReplacement(raw, format, "needle", "X", "all", true, () => true);
      expect(plan.replaced).toBe(n);
      expect(renderedBlockTextCallCountForTests() - renders).toBe(2);
      expect(calls.mapping).toBe(1);
    });
  }
});
