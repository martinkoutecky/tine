import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { createEffect, createRoot } from "solid-js";
import { backend } from "../backend";
import { bindingOwner } from "../owned";
import { initParser } from "../render/parse";
import { parseBody } from "../render/facets";
import type { PageRead } from "../types";
import { applyGraphChangesBulk } from "./external";
import { doc } from "./model";
import { admitRead } from "./readAdmission";
import { ensurePageLoaded, loadFeed, resetStore } from "./workingSet";

beforeAll(initParser);
afterEach(() => { resetStore(); vi.restoreAllMocks(); });

const page = (name: string, blocks: number, revision = "initial"): PageRead => ({
  name, title: name, kind: "page", id: `pages/${name}.md`, rev: revision, pre_block: null,
  blocks: Array.from({ length: blocks }, (_, i) => ({ id: `${name}-${i}`,
    raw: `TODO ${name} ${i} ${revision} [[Linked page]] **synthetic content**`
      + (name === "Target" && i === 0 ? "\nSynthetic [[linked]] **body** with `inline code`.".repeat(256) : ""),
    collapsed: false, children: [] })),
});

// Same sampling as npm run bench: discard one warmup, retain all eight
// samples, gate their minimum to avoid charging another lane's scheduler pause.
async function measure(label: string, run: () => Promise<void>): Promise<number> {
  await run();
  const samples: number[] = [];
  for (let i = 0; i < 8; i++) {
    const start = performance.now();
    await run();
    samples.push(performance.now() - start);
  }
  console.log(`${label} ms: ${samples.map(ms => ms.toFixed(3)).join(", ")}; min=${Math.min(...samples).toFixed(3)}`);
  return Math.min(...samples);
}

function fixture() {
  const target = page("Target", 2000);
  const others = Array.from({ length: 128 }, (_, i) => page(`Other-${i}`, 256));
  // Feed membership retains every synthetic open page beyond the working-set cap.
  loadFeed([...others, target]);
  return { target, others };
}

it("admits a warm 2000-block page among 129 open pages within 16 ms", async () => {
  const { target } = fixture();
  const read = vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => structuredClone(target));
  const owner = bindingOwner();
  const elapsed = await measure("warm admission", async () => {
    const result = await admitRead(owner, () => backend().getPageByPath(target.id), dto => [dto!.name], dto => ensurePageLoaded(dto!));
    expect(result).toEqual({ kind: "current", value: null });
  });
  expect(elapsed).toBeLessThan(16);
  expect(read).toHaveBeenCalledTimes(9);
  expect(doc.byId["Target-0"].raw).toBe(target.blocks[0].raw);
});

it("delivers one 2000-block publication with 192 live views within 100 ms", async () => {
  const { target, others } = fixture();
  const revisions = Array.from({ length: 9 }, (_, i) => page("Target", 2000, String(i + 1)));
  const answers = new Map([target, ...others].map(dto => [dto.id, dto]));
  const read = vi.spyOn(backend(), "getPageByPath").mockImplementation(async path => structuredClone(answers.get(path)!));
  const named = vi.spyOn(backend(), "getPage");
  const visible = new Map<string, string>();
  let targetRenders = 0, otherRenders = 0;
  const dispose = createRoot(dispose => {
    // Multiple panes/embeds of one live block share the parser's retained body.
    for (let i = 0; i < 64; i++) createEffect(() => {
      const raw = doc.byId["Target-0"].raw;
      parseBody(raw, "md");
      visible.set(`target-${i}`, raw);
      targetRenders++;
    });
    for (const dto of others) createEffect(() => {
      const raw = doc.byId[dto.blocks[0].id].raw;
      parseBody(raw, "md");
      visible.set(dto.name, raw);
      otherRenders++;
    });
    return dispose;
  });
  try {
    const initialOthers = otherRenders;
    let revision = 0;
    const elapsed = await measure("one-page publication", async () => {
      const rev = String(++revision);
      answers.set(target.id, revisions[revision - 1]);
      await applyGraphChangesBulk({ changes: [], answers: { rev, inventoryChanged: false, blockRefCounts: {},
        summary: { rev, pages: ["target"], blocks: [], linksTo: [], props: {}, inventoryChanged: false, full: false } } });
    });
    expect.soft(elapsed).toBeLessThan(100);
    expect(read).toHaveBeenCalledTimes(9);
    expect(named).not.toHaveBeenCalled();
    expect(otherRenders).toBe(initialOthers);
    expect(targetRenders).toBe(64 * 10);
    for (let i = 0; i < 64; i++) expect(visible.get(`target-${i}`)).toBe(answers.get(target.id)!.blocks[0].raw);
  } finally { dispose(); }
});
