import { afterEach, expect, it } from "vitest";
import { createComputed, createRoot } from "solid-js";
import { produce } from "solid-js/store";
import { doc, setDoc } from "./model";
import { ensurePageLoaded, forgetPage, loadSingle, reloadPage, resetStore } from "./workingSet";
import { resolveBlockRef } from "./edits/identity";

afterEach(resetStore);

it.each([8, 32])("changing one claim leaves %i different identities on the same page asleep", size => {
  loadSingle({ name: "Claims", title: "Claims", kind: "page", pre_block: null, format: "md", id: "pages/Claims.md",
    blocks: Array.from({ length: size + 1 }, (_, i) => ({ id: `b${i}`, raw: `Body\nid:: claim${i}`, collapsed: false, children: [] })) });
  const ref = (uuid: string) => ({ uuid, page: "Claims", pageKind: "page" as const, path: "pages/Claims.md" });
  let other = 0;
  const answers: (string | null)[] = [];
  const dispose = createRoot(dispose => {
    for (let i = 1; i <= size; i++) createComputed(() => { resolveBlockRef(ref(`claim${i}`)); other++; });
    createComputed(() => { answers.push(resolveBlockRef(ref("fresh"))); });
    return dispose;
  });
  try {
    other = 0;
    setDoc("byId", "b0", "raw", "Body\nid:: fresh");
    expect(other).toBe(0);
    expect(answers.at(-1)).toBe("b0");
    setDoc("byId", "b1", "raw", "Body\nid:: fresh");
    expect(answers.at(-1)).toBeNull();
    setDoc("byId", "b0", "raw", "Body");
    expect(answers.at(-1)).toBe("b1");
  } finally { dispose(); }
});

it.each(["md", "org"] as const)("keeps %s zoom/reference identities live across stamping, duplicates, undo and replacement", format => {
  const raw = (id: string) => format === "md" ? `Body\nid:: ${id}` : `Body\n:PROPERTIES:\n:ID: ${id}\n:END:`;
  const page = {
    name: "Indexed", title: "Indexed", kind: "page" as const, pre_block: null, format,
    id: `pages/Indexed.${format}`,
    blocks: ["one", "two"].map(id => ({ id, raw: "Body", collapsed: false, children: [] })),
  };
  loadSingle(page);
  const ref = { uuid: "claimed", page: "Indexed", pageKind: "page" as const, path: page.id };
  const observed: (string | null)[] = [];
  const dispose = createRoot(dispose => {
    createComputed(() => observed.push(resolveBlockRef(ref, { navigation: true })));
    return dispose;
  });
  try {
    expect(observed.at(-1)).toBeNull();
    setDoc("byId", "one", "raw", raw("claimed"));
    expect(observed.at(-1)).toBe("one");
    setDoc("byId", "two", "raw", raw("claimed"));
    expect(observed.at(-1)).toBeNull();
    setDoc("byId", "one", "raw", "Body");
    expect(observed.at(-1)).toBe("two");
    // Undo and structural edits use producers rather than raw-only writes.
    setDoc(produce(state => { state.byId.two.raw = "Body"; state.byId.one.raw = raw("claimed"); }));
    expect(observed.at(-1)).toBe("one");
    setDoc("pages", 0, "roots", ["two"]);
    expect(observed.at(-1)).toBeNull(); // an orphan is not a claimant
    setDoc("pages", 0, "roots", ["one", "two"]);
    expect(observed.at(-1)).toBe("one");
    expect(resolveBlockRef({ ...ref, pageKind: "journal" })).toBeNull();
    expect(resolveBlockRef({ ...ref, path: "pages/other.md" })).toBeNull();
    expect(resolveBlockRef({ ...ref, uuid: "one" })).toBeNull();
    expect(resolveBlockRef({ ...ref, uuid: "one" }, { navigation: true })).toBe("one");
    resetStore();
    loadSingle({ ...page, blocks: [{ ...page.blocks[1], raw: raw("claimed") }] });
    expect(observed.at(-1)).toBe("two");
    expect(doc.byId.one).toBeUndefined();
  } finally { dispose(); }
});

it("loading, forgetting or evicting other pages leaves a page's identity answer alone (GH #886)", () => {
  const page = (name: string, raw = "Body") => ({
    name, title: name, kind: "page" as const, pre_block: null, format: "md" as const, id: `pages/${name}.md`,
    blocks: [{ id: `${name}-b`, raw, collapsed: false, children: [] }],
  });
  for (let i = 0; i < 79; i++) expect(ensurePageLoaded(page(`Other ${i}`))).toBeNull();
  expect(ensurePageLoaded(page("Source", "Body\nid:: claimed"))).toBeNull();
  const ref = { uuid: "claimed", page: "Source", pageKind: "page" as const, path: "pages/Source.md" };
  let runs = 0;
  const observed: (string | null)[] = [];
  const dispose = createRoot(dispose => {
    createComputed(() => { runs++; observed.push(resolveBlockRef(ref)); });
    return dispose;
  });
  try {
    expect(observed.at(-1)).toBe("Source-b");
    const before = runs;
    // Each of these used to clear every page's index and re-run every
    // mounted reference group's resolver: N loads x N groups while scrolling.
    expect(ensurePageLoaded(page("Extra"))).toBeNull(); // 81 pages: evicts "Other 0"
    expect(doc.pages.some(p => p.name === "Other 0")).toBe(false);
    forgetPage("Extra");
    expect(ensurePageLoaded(page("Other 0"))).toBeNull();
    expect(runs).toBe(before);
    // The page's own reload still re-answers.
    reloadPage(page("Source", "Body"));
    expect(observed.at(-1)).toBeNull();
    reloadPage(page("Source", "Body\nid:: claimed"));
    expect(observed.at(-1)).toBe("Source-b");
    forgetPage("Source");
    expect(observed.at(-1)).toBeNull();
  } finally { dispose(); }
});
