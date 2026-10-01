import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: page keys and canonical group tokens have one native owner, shared with WASM", () => {
  const read = (path: string) => readFileSync(path, "utf8");
  const page = read("src/pageIdentity.ts");
  const group = read("src/editor/queryViewProperties.ts");
  const wasm = read("crates/lsdoc-wasm/src/lib.rs");
  expect(page, "I-12: use refs::page_key via page_identity_key; exemplar src/pageIdentity.ts").toContain("page_identity_key");
  expect(page).not.toMatch(/\.trim\(|\.toLowerCase\(|\.normalize\(/);
  expect(group, "I-12: canonicalGroupField uses query/group_field.rs via WASM").toContain("canonical_group_field(value)");
  expect(group).not.toContain("QUERY_COLUMN_BUILTINS");
  expect(group).not.toContain('value.trim()');
  expect(wasm).toContain('mod page_identity;');
  expect(wasm).toContain('mod group_field;');
  expect(wasm).toContain('page_identity::page_key(name)');
  expect(wasm).toContain('group_field::canonical_group_token(value)');
  expect(read("crates/tine-core/src/refs.rs")).toContain('pub use page_identity::page_key;');
  expect(read("crates/tine-core/src/query/view.rs")).toContain('group_field::canonical_group_token(value).map(Field::new)');
});

it("I-4: session restore waits for the WASM parser that owns page identity and group fields", () => {
  const main = readFileSync("src/main.tsx", "utf8");
  expect(main, "restoreSession parses saved query views through synchronous WASM; start it only after initParser settles (src/main.tsx)")
    .toMatch(/parserSettled\.then\(\(\) => Promise\.race\(\[restoreSession\(\)/);
  expect(main.match(/restoreSession\(\)/g)?.length).toBe(1);
});
