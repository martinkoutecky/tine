import { afterEach, expect, it } from "vitest";
import { createComputed, createRoot } from "solid-js";
import { produce } from "solid-js/store";
import { doc, setDoc } from "./model";
import { loadSingle, resetStore } from "./workingSet";
import { resolveBlockRef } from "./edits/identity";

afterEach(resetStore);

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
