// The folded "N below" count reads the comment index (review rounds 2-3, PC-4)
// and keeps the imperative count's answer through every
// kind of change: resolve, reopen, a hidden block gaining or losing its quote, a
// reply, structural moves, and the folded block itself joining a thread.
import { afterEach, beforeAll, beforeEach, expect, it } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { initParser, blockRegions } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { resetStore, setRaw } from "./document";
import { indentBlock, outdentBlock, deleteBlock } from "./document/edits/blocks";
import { setThreadResolved } from "./commentActions";
import { descendantOpenThreads } from "./commentGroups";
import { createFoldedThreadCount } from "./commentBelowCount";
import { bindTestHost } from "./document/host/wiring.test.support";
import { answerOpensFromDocument } from "./document/host/documentHost.test.support";
import type { BlockDto } from "./types";

function b(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, children, collapsed: false, properties: blockRegions(raw, "md").properties.filter(p => p.primary).map(p => [p.key, p.value]) };
}
let dispose: (() => void) | undefined;
beforeAll(async () => { await initParser(); });
beforeEach(async () => { answerOpensFromDocument(await bindTestHost()); });
afterEach(() => { dispose?.(); dispose = undefined; resetStore(); });

it("keeps the imperative answer through text, resolution and structural changes", () => {
  loadSingle({ name: "Below", title: "Below", kind: "page", pre_block: null, format: "md", blocks: [
    b("top", "Top", [b("p", "Folded parent", [
      b("own", "Own thread\ntine.quote:: Folded"),
      b("body", "Body passage", [b("t1", "Thread one\ntine.quote:: Body", [b("r1", "A reply\ntine.quote:: passage")]), b("plain", "Plain child")]),
      b("body2", "Second body", [b("deep", "Deeper", [b("t2", "Thread two\ntine.quote:: Deeper")])]),
    ])]),
  ] });
  const [active, setActive] = createSignal(true);
  const count = createRoot(root => { dispose = root; return createFoldedThreadCount(() => "p", active); });
  const same = (expected: number) => { expect(count()).toBe(expected); expect(descendantOpenThreads("p", "md")).toBe(expected); };
  same(2);
  setRaw("p", "Folded parent, edited", { timetracking: false }); same(2);
  setRaw("r1", "A reply edited\ntine.quote:: passage", { timetracking: false }); same(2);
  setThreadResolved("t1", true); same(1);
  setThreadResolved("t1", false); same(2);
  setRaw("plain", "Plain child\ntine.quote:: Body", { timetracking: false }); same(3);
  setRaw("plain", "Plain child again", { timetracking: false }); same(2);
  setRaw("t2", "No longer a thread", { timetracking: false }); same(1);
  setRaw("t2", "Thread two again\ntine.quote:: Deeper", { timetracking: false }); same(2);
  // Structure: hidden subtrees move between levels, and a counted thread is deleted.
  outdentBlock("deep", 0); same(2);
  deleteBlock("t1"); same(1);
  indentBlock("body2", 0); same(1);
  // The folded block joining a thread counts nothing below it.
  setRaw("p", "Folded parent\ntine.quote:: Top", { timetracking: false }); same(0);
  setRaw("p", "Folded parent", { timetracking: false }); same(1);
  setActive(false); expect(count()).toBe(0);
  setActive(true); same(1);
  deleteBlock("deep"); same(0);
});
